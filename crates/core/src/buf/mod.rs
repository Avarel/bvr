pub mod shard;

use std::{num::NonZeroUsize, rc::Rc};

use anyhow::Result;
use lru::LruCache;
use tokio::{fs::File, sync::mpsc::Receiver};

use self::shard::{Shard, ShardStr};
use crate::index::{
    inflight::{InflightIndex, InflightIndexMode, InflightIndexProgress, InflightStream},
    CompleteIndex, FileIndex,
};

pub struct ShardedBuffer<Idx> {
    index: Idx,
    shards: Repr,
}

enum Repr {
    /// Data can be loaded on demand
    /// Shard boundaries are line boundaries
    File(LruShardedFile),
    /// Data is all present in memory in multiple mmaps
    /// All shards are assumed to have the same sizes
    Stream {
        pending_shards: Option<Receiver<Shard>>,
        shards: Vec<Rc<Shard>>,
    },
}

impl ShardedBuffer<CompleteIndex> {
    pub async fn read_file(file: File, shard_count: usize) -> Result<Self> {
        let (mut index, indexer) = InflightIndex::new(InflightIndexMode::File);
        indexer.index_file(file.try_clone().await?).await?;
        assert!(index.try_finalize());

        Ok(Self {
            index: index.unwrap(),
            shards: Repr::File(LruShardedFile {
                file,
                shards: LruCache::new(NonZeroUsize::new(shard_count).unwrap()),
            }),
        })
    }

    pub async fn read_stream(stream: InflightStream) -> Result<Self> {
        let (mut index, indexer) = InflightIndex::new(InflightIndexMode::Stream);
        let (sx, rx) = tokio::sync::mpsc::channel(1024);
        indexer.index_stream(stream, sx).await?;
        assert!(index.try_finalize());

        Ok(Self {
            index: index.unwrap(),
            shards: Repr::Stream {
                pending_shards: Some(rx),
                shards: Vec::new(),
            },
        })
    }
}

impl ShardedBuffer<InflightIndex> {
    pub async fn read_file(file: File, shard_count: usize) -> Result<Self> {
        let (index, indexer) = InflightIndex::new(InflightIndexMode::File);
        tokio::spawn(indexer.index_file(file.try_clone().await?));

        Ok(Self {
            index,
            shards: Repr::File(LruShardedFile {
                file,
                shards: LruCache::new(NonZeroUsize::new(shard_count).unwrap()),
            }),
        })
    }

    pub async fn read_stream(stream: InflightStream) -> Result<Self> {
        let (index, indexer) = InflightIndex::new(InflightIndexMode::Stream);
        let (sx, rx) = tokio::sync::mpsc::channel(1024);
        tokio::spawn(indexer.index_stream(stream, sx));

        Ok(Self {
            index,
            shards: Repr::Stream {
                pending_shards: Some(rx),
                shards: Vec::new(),
            },
        })
    }

    pub fn try_finalize(&mut self) -> bool {
        self.index.try_finalize()
    }

    pub fn progress(&self) -> InflightIndexProgress {
        self.index.progress()
    }
}

trait ShardContainer {
    fn fetch(&mut self, shard_id: usize) -> Result<Rc<Shard>>;
    fn cap(&self) -> usize;
}

impl ShardContainer for &mut Vec<Rc<Shard>> {
    fn fetch(&mut self, shard_id: usize) -> Result<Rc<Shard>> {
        Ok(self[shard_id].clone())
    }

    fn cap(&self) -> usize {
        self.len()
    }
}

struct LruShardedFile {
    file: File,
    shards: LruCache<usize, Rc<Shard>>,
}

impl ShardContainer for &mut LruShardedFile {
    fn fetch(&mut self, shard_id: usize) -> Result<Rc<Shard>> {
        let range = {
            let shard_id = shard_id as u64;
            (shard_id * crate::INDEXING_VIEW_SIZE)..((shard_id + 1) * crate::INDEXING_VIEW_SIZE)
        };
        self.shards
            .try_get_or_insert(shard_id, || {
                Ok::<Rc<Shard>, anyhow::Error>(Rc::new(Shard::map_file(
                    shard_id, range, &self.file,
                )?))
            })
            .cloned()
    }

    fn cap(&self) -> usize {
        self.shards.cap().get()
    }
}

impl<Idx: FileIndex> ShardedBuffer<Idx> {
    pub fn line_count(&self) -> usize {
        self.index.line_count()
    }

    fn fetch_line(
        index: &Idx,
        mut container: impl ShardContainer,
        line_number: usize,
    ) -> Result<ShardStr> {
        let data_start = index.start_of_line(line_number);
        let data_end = index.start_of_line(line_number + 1);
        let shard_start = (data_start / crate::INDEXING_VIEW_SIZE) as usize;
        let shard_end = (data_end / crate::INDEXING_VIEW_SIZE) as usize;

        if shard_start == shard_end {
            // The data is in a single shard
            let shard = container.fetch(shard_start as usize)?;
            let (start, end) = shard.translate_inner_data_range(data_start, data_end);
            Ok(shard.get_shard_line(start, end))
        } else {
            debug_assert!(shard_start < shard_end);
            assert!(shard_end - shard_start + 1 > container.cap());
            // The data may cross several shards, so we must piece together
            // the data from across the shards.
            let mut buf = Vec::with_capacity((data_end - data_start) as usize);

            let shard_first = container.fetch(shard_start as usize)?;
            let shard_last = container.fetch(shard_start as usize)?;
            let (start, end) = (
                shard_first.translate_inner_data_index(data_start),
                shard_last.translate_inner_data_index(data_end),
            );
            buf.extend_from_slice(&shard_first[start as usize..]);
            for shard_id in shard_start + 1..shard_end {
                buf.extend_from_slice(&container.fetch(shard_id)?);
            }
            buf.extend_from_slice(&shard_last[..end as usize]);

            let buf = String::from_utf8_lossy(&buf).into_owned();
            Ok(ShardStr::new_owned(buf))
        }
    }

    pub fn get_line(&mut self, line_number: usize) -> Result<ShardStr> {
        assert!(line_number <= self.line_count());
        match &mut self.shards {
            Repr::File(file) => Self::fetch_line(&self.index, file, line_number),
            Repr::Stream {
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

                Self::fetch_line(&self.index, shards, line_number)
            }
        }
    }
}

#[cfg(test)]
mod test {
    use std::io::BufReader;

    use crate::{buf::ShardedBuffer, index::CompleteIndex};

    #[test]
    fn what() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let file = rt.block_on(tokio::fs::File::open("./Cargo.toml")).unwrap();
        let mut file = rt
            .block_on(ShardedBuffer::<CompleteIndex>::read_file(file, 25))
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
            .block_on(ShardedBuffer::<CompleteIndex>::read_file(file, 25))
            .unwrap();
        let mut stream_index = rt
            .block_on(ShardedBuffer::<CompleteIndex>::read_stream(Box::new(stream)))
            .unwrap();

        assert_eq!(file_index.line_count(), stream_index.line_count());
        for i in 0..file_index.line_count() {
            assert_eq!(
                file_index.get_line(i).unwrap().as_str(),
                stream_index.get_line(i).unwrap().as_str()
            );
        }
    }
}
