pub mod shard;

use std::{
    fs::File,
    num::NonZeroUsize,
    rc::Rc,
    sync::mpsc::{Receiver, TryRecvError},
};

use anyhow::Result;
use lru::LruCache;

use self::shard::{Shard, ShardStr};
use crate::index::{
    inflight::{InflightIndex, InflightIndexMode, InflightIndexProgress, Stream},
    BufferIndex, CompleteIndex,
};

pub struct ShardedBuffer<Idx> {
    /// The [BufferIndex] of this buffer.
    index: Idx,
    /// The internal representation of this buffer.
    shards: Repr,
}

/// Internal representation of the sharded buffer, which allows for working
/// with both files and streams of data. All shards are assumed to have
/// the same size with the exception of the last shard.
enum Repr {
    /// Data can be loaded on demand.
    File(LruShardedFile),
    /// Data is all present in memory in multiple anonymous mmaps.
    Stream {
        pending_shards: Option<Receiver<Shard>>,
        shards: Vec<Rc<Shard>>,
    },
}

impl ShardedBuffer<CompleteIndex> {
    /// Read a [ShardedBuffer] from a [File].
    pub fn read_file(file: File, shard_count: usize) -> Result<Self> {
        let (mut index, indexer) = InflightIndex::new(InflightIndexMode::File);
        indexer.index_file(file.try_clone()?)?;
        assert!(index.try_finalize());

        Ok(Self {
            index: index.unwrap(),
            shards: Repr::File(LruShardedFile {
                len: file.metadata()?.len(),
                file,
                shards: LruCache::new(NonZeroUsize::new(shard_count).unwrap()),
            }),
        })
    }

    /// Read a [ShardedBuffer] from an [Stream].
    pub fn read_stream(stream: Stream) -> Result<Self> {
        let (mut index, indexer) = InflightIndex::new(InflightIndexMode::Stream);
        let (sx, rx) = std::sync::mpsc::channel();
        indexer.index_stream(stream, sx)?;
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
    /// Read a [ShardedBuffer] from a [File].
    pub fn read_file(file: File, shard_count: usize) -> Result<Self> {
        let (index, indexer) = InflightIndex::new(InflightIndexMode::File);
        std::thread::spawn({
            let file = file.try_clone()?;
            move || indexer.index_file(file)
        });

        Ok(Self {
            index,
            shards: Repr::File(LruShardedFile {
                len: file.metadata()?.len(),
                file,
                shards: LruCache::new(NonZeroUsize::new(shard_count).unwrap()),
            }),
        })
    }

    /// Read a [ShardedBuffer] from an [Stream].
    pub fn read_stream(stream: Stream) -> Result<Self> {
        let (index, indexer) = InflightIndex::new(InflightIndexMode::Stream);
        let (sx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || indexer.index_stream(stream, sx));

        Ok(Self {
            index,
            shards: Repr::Stream {
                pending_shards: Some(rx),
                shards: Vec::new(),
            },
        })
    }

    /// Attempt to finalize the inner [InflightIndex].
    pub fn try_finalize(&mut self) -> bool {
        self.index.try_finalize()
    }

    /// Report the progress of the inner [InflightIndex].
    pub fn progress(&self) -> InflightIndexProgress {
        self.index.progress()
    }
}

trait ShardContainer {
    fn fetch(&mut self, shard_id: usize) -> Result<Rc<Shard>>;
    fn cap(&self) -> usize;
}

impl ShardContainer for &[Rc<Shard>] {
    fn fetch(&mut self, shard_id: usize) -> Result<Rc<Shard>> {
        Ok(self[shard_id].clone())
    }

    fn cap(&self) -> usize {
        self.len()
    }
}

struct LruShardedFile {
    file: File,
    len: u64,
    shards: LruCache<usize, Rc<Shard>>,
}

impl ShardContainer for &mut LruShardedFile {
    fn fetch(&mut self, shard_id: usize) -> Result<Rc<Shard>> {
        let range = {
            let shard_id = shard_id as u64;
            (shard_id * crate::SHARD_SIZE)..((shard_id + 1) * crate::SHARD_SIZE).min(self.len)
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

impl<Idx> ShardedBuffer<Idx>
where
    Idx: BufferIndex,
{
    /// Return the line count of this [ShardedBuffer].
    pub fn line_count(&self) -> usize {
        self.index.line_count()
    }

    fn fetch_line(
        index: &Idx,
        mut container: impl ShardContainer,
        line_number: usize,
    ) -> Result<ShardStr> {
        let data_start = index.data_of_line(line_number).unwrap();
        let data_end = index.data_of_line(line_number + 1).unwrap();
        let shard_start = (data_start / crate::SHARD_SIZE) as usize;
        let shard_end = (data_end / crate::SHARD_SIZE) as usize;

        if shard_start == shard_end {
            // The data is in a single shard
            let shard = container.fetch(shard_start as usize)?;
            let (start, end) = shard.translate_inner_data_range(data_start, data_end);
            Ok(shard.get_shard_line(start, end))
        } else {
            debug_assert!(shard_start < shard_end);
            assert!(shard_end - shard_start + 1 <= container.cap());
            // The data may cross several shards, so we must piece together
            // the data from across the shards.
            let mut buf = Vec::with_capacity((data_end - data_start) as usize);

            let shard_first = container.fetch(shard_start as usize)?;
            let shard_last = container.fetch(shard_end as usize)?;
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

    /// Get a [ShardStr] from this [ShardedBuffer].
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
                            Err(TryRecvError::Empty) => break,
                            Err(TryRecvError::Disconnected) => {
                                *pending_shards = None;
                                break;
                            }
                        }
                    }
                }

                Self::fetch_line(&self.index, shards.as_slice(), line_number)
            }
        }
    }
}

#[cfg(test)]
mod test {
    use anyhow::Result;
    use std::{fs::File, io::BufReader};

    use crate::{buf::ShardedBuffer, index::CompleteIndex};

    #[test]
    fn what() {
        let file = std::fs::File::open("./Cargo.toml").unwrap();
        let mut file = ShardedBuffer::<CompleteIndex>::read_file(file, 25).unwrap();
        dbg!(file.line_count());

        for i in 0..file.line_count() {
            eprintln!("{}\t{}", i + 1, file.get_line(i).unwrap());
        }
    }

    #[test]
    fn file_stream_consistency_1() -> Result<()> {
        file_stream_consistency_base(File::open("../../tests/test_10.log")?, 10)
    }

    #[test]
    fn file_stream_consistency_2() -> Result<()> {
        file_stream_consistency_base(File::open("../../tests/test_50_long.log")?, 50)
    }

    #[test]
    fn file_stream_consistency_3() -> Result<()> {
        file_stream_consistency_base(File::open("../../tests/test_5000000.log")?, 5_000_000)
    }

    fn file_stream_consistency_base(file: File, line_count: usize) -> Result<()> {
        let stream = BufReader::new(file.try_clone()?);

        let mut file_index = ShardedBuffer::<CompleteIndex>::read_file(file, 25)?;
        let mut stream_index = ShardedBuffer::<CompleteIndex>::read_stream(Box::new(stream))?;

        assert_eq!(file_index.line_count(), stream_index.line_count());
        assert_eq!(file_index.line_count(), line_count);
        for i in 0..file_index.line_count() {
            assert_eq!(
                file_index.get_line(i).unwrap().as_str(),
                stream_index.get_line(i).unwrap().as_str()
            );
        }

        Ok(())
    }
}
