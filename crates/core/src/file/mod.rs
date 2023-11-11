pub mod shard;

use std::{num::NonZeroUsize, rc::Rc};

use anyhow::Result;
use lru::LruCache;
use tokio::{fs::File, sync::mpsc::Receiver};

use self::shard::{Shard, ShardStr};
use crate::index::{
    sync::{AsyncIndex, AsyncIndexMode, AsyncIndexProgress, AsyncStream},
    CompleteIndex, FileIndex,
};

pub struct ShardedFile<Idx> {
    index: Idx,
    shards: ShardRepr,
}

enum ShardRepr {
    /// Data can be loaded on demand
    /// Shard boundaries are line boundaries
    File {
        file: File,
        shards: LruCache<usize, Rc<Shard>>,
    },
    /// Data is all present in memory in multiple mmaps
    /// All shards are assumed to have the same sizes
    Stream {
        pending_shards: Option<Receiver<Shard>>,
        shards: Vec<Rc<Shard>>,
    },
}

impl ShardedFile<CompleteIndex> {
    pub async fn read_file(file: File, shard_count: usize) -> Result<Self> {
        let (mut index, indexer) = AsyncIndex::new(AsyncIndexMode::File);
        indexer.index_file(file.try_clone().await?).await?;
        assert!(index.try_finalize());

        Ok(Self {
            index: index.unwrap(),
            shards: ShardRepr::File {
                file,
                shards: LruCache::new(NonZeroUsize::new(shard_count).unwrap()),
            },
        })
    }

    pub async fn read_stream(stream: AsyncStream) -> Result<Self> {
        let (mut index, indexer) = AsyncIndex::new(AsyncIndexMode::Stream);
        let (sx, rx) = tokio::sync::mpsc::channel(1024);
        indexer.index_stream(stream, sx).await?;
        assert!(index.try_finalize());

        Ok(Self {
            index: index.unwrap(),
            shards: ShardRepr::Stream {
                pending_shards: Some(rx),
                shards: Vec::new(),
            },
        })
    }
}

impl ShardedFile<AsyncIndex> {
    pub async fn read_file(file: File, shard_count: usize) -> Result<Self> {
        let (index, indexer) = AsyncIndex::new(AsyncIndexMode::File);
        tokio::spawn(indexer.index_file(file.try_clone().await?));

        Ok(Self {
            index,
            shards: ShardRepr::File {
                file,
                shards: LruCache::new(NonZeroUsize::new(shard_count).unwrap()),
            },
        })
    }

    pub async fn read_stream(stream: AsyncStream) -> Result<Self> {
        let (index, indexer) = AsyncIndex::new(AsyncIndexMode::Stream);
        let (sx, rx) = tokio::sync::mpsc::channel(1024);
        tokio::spawn(indexer.index_stream(stream, sx));

        Ok(Self {
            index,
            shards: ShardRepr::Stream {
                pending_shards: Some(rx),
                shards: Vec::new(),
            },
        })
    }

    pub fn try_finalize(&mut self) -> bool {
        self.index.try_finalize()
    }

    pub fn progress(&self) -> AsyncIndexProgress {
        self.index.progress()
    }
}

impl<Idx: FileIndex> ShardedFile<Idx> {
    pub fn line_count(&self) -> usize {
        self.index.line_count()
    }

    pub fn get_line(&mut self, line_number: usize) -> Result<ShardStr> {
        assert!(line_number <= self.line_count());
        match &mut self.shards {
            ShardRepr::File { file, shards } => {
                let shard_id = self.index.shard_of_line(line_number).unwrap();

                let range = self.index.data_range_of_shard(shard_id).unwrap();
                let shard = shards
                    .try_get_or_insert(shard_id, || {
                        Ok::<Rc<Shard>, anyhow::Error>(Rc::new(Shard::map_file(
                            shard_id, range, file,
                        )?))
                    })
                    .cloned()?;

                let (start, end) = shard.translate_inner_data_range(
                    self.index.start_of_line(line_number),
                    self.index.start_of_line(line_number + 1),
                );
                // Trim the newline
                let start = if line_number == 0 { 0 } else { start + 1 };
                Ok(shard.get_shard_line(start, end))
            }
            ShardRepr::Stream {
                pending_shards,
                shards,
            } => {
                if let Some(rx) = pending_shards {
                    loop {
                        match rx.try_recv() {
                            Ok(shard) => {
                                assert_eq!(shard.id(), shards.len());
                                shards.push(Rc::new(shard))
                            }
                            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                                *pending_shards = None;
                                break;
                            }
                        }
                    }
                }

                let data_start = self.index.start_of_line(line_number);
                let data_end = self.index.start_of_line(line_number + 1);
                let shard_start = data_start / crate::INDEXING_VIEW_SIZE;
                let shard_end = data_end / crate::INDEXING_VIEW_SIZE;

                assert!(shard_start < shards.len() as u64);
                assert!(shard_end < shards.len() as u64);
                if shard_start == shard_end {
                    // The data is in a single shard
                    let shard = &shards[shard_start as usize];
                    let (start, end) = shard.translate_inner_data_range(data_start, data_end);
                    Ok(shard.get_shard_line(start, end))
                } else {
                    debug_assert!(shard_start < shard_end);
                    // The data may cross several shards, so we must piece together
                    // the data from across the shards.
                    let mut buf = Vec::with_capacity((data_end - data_start) as usize);

                    let shard_first = &shards[shard_start as usize];
                    let shard_last = &shards[shard_end as usize];
                    let (start, end) = (
                        shard_first.translate_inner_data_index(data_start),
                        shard_last.translate_inner_data_index(data_end)
                    );
                    buf.extend_from_slice(&shard_first[start as usize..]);
                    for shard_id in shard_start + 1..shard_end {
                        buf.extend_from_slice(&shards[shard_id as usize]);
                    }
                    buf.extend_from_slice(&shard_last[..end as usize]);

                    let buf = String::from_utf8_lossy(&buf).into_owned();
                    Ok(ShardStr::new_owned(buf))
                }
            }
        }
    }
}

#[cfg(test)]
mod test {
    use std::io::BufReader;

    use crate::{file::ShardedFile, index::CompleteIndex};

    #[test]
    fn what() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let file = rt.block_on(tokio::fs::File::open("./Cargo.toml")).unwrap();
        let mut file = rt
            .block_on(ShardedFile::<CompleteIndex>::read_file(file, 25))
            .unwrap();
        dbg!(file.line_count());

        for i in 0..file.line_count() {
            eprintln!("{}\t{}", i + 1, file.get_line(i).unwrap());
        }
    }

    #[test]
    fn file_stream_consistency() {
        let path = "./Cargo.toml";
        let rt = tokio::runtime::Runtime::new().unwrap();
        let file = std::fs::File::open(path).unwrap();
        let stream = BufReader::new(file);
        let file = rt.block_on(tokio::fs::File::open(path)).unwrap();

        let mut file_index = rt
            .block_on(ShardedFile::<CompleteIndex>::read_file(file, 25))
            .unwrap();
        let mut stream_index = rt
            .block_on(ShardedFile::<CompleteIndex>::read_stream(Box::new(stream)))
            .unwrap();

        assert_eq!(file_index.line_count(), stream_index.line_count());
        for i in 0..file_index.line_count() {
            assert_eq!(file_index.get_line(i).unwrap().as_str(), stream_index.get_line(i).unwrap().as_str());
        }
    }
}
