//! The `buf` module contains the [ShardedBuffer] struct, which is the main
//! interface for creating and interacting with the sharded buffers.

pub mod shard;

use std::{
    fs::File,
    num::NonZeroUsize,
    ops::Range,
    sync::{
        mpsc::{Receiver, TryRecvError},
        Arc,
    },
};

use crate::Result;
use lru::LruCache;

use self::shard::{Shard, ShardStr};
use crate::index::{
    inflight::{InflightIndex, InflightIndexMode, InflightIndexProgress, Stream},
    BufferIndex, CompleteIndex,
};

/// A sharded buffer that holds data in multiple shards.
///
/// The `ShardedBuffer` struct represents a buffer that is divided into multiple shards.
/// It contains the [BufferIndex] and the internal representation of the shards.
pub struct ShardedBuffer<Idx> {
    /// The [BufferIndex] of this buffer.
    index: Idx,
    /// The internal representation of this buffer.
    shards: ShardRepr,
}

/// Internal representation of the sharded buffer, which allows for working
/// with both files and streams of data. All shards are assumed to have
/// the same size with the exception of the last shard.
enum ShardRepr {
    /// Data can be loaded on demand.
    File {
        file: File,
        len: u64,
        shards: LruCache<usize, Arc<Shard>>,
    },
    /// Data is all present in memory in multiple anonymous mmaps.
    Stream {
        pending_shards: Option<Receiver<Shard>>,
        shards: Vec<Arc<Shard>>,
    },
}

impl ShardRepr {
    fn fetch(&mut self, shard_id: usize) -> Arc<Shard> {
        match self {
            ShardRepr::File { file, len, shards } => {
                let range = {
                    let shard_id = shard_id as u64;
                    (shard_id * crate::SHARD_SIZE)..((shard_id + 1) * crate::SHARD_SIZE).min(*len)
                };
                shards
                    .get_or_insert(shard_id, || {
                        Arc::new(Shard::map_file(shard_id, range, file))
                    })
                    .clone()
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
                                shards.push(Arc::new(shard))
                            }
                            Err(TryRecvError::Empty) => break,
                            Err(TryRecvError::Disconnected) => {
                                *pending_shards = None;
                                break;
                            }
                        }
                    }
                }
                shards[shard_id].clone()
            }
        }
    }
}

impl ShardedBuffer<CompleteIndex> {
    /// Reads a file and constructs a new instance of [ShardedBuffer].
    ///
    /// This function uses [InflightIndex] with [InflightIndexMode::File], which
    /// then it uses to completely index the file. The index is then finalized and
    /// the resulting [CompleteIndex] is used to construct the [ShardedBuffer].
    /// 
    /// # Arguments
    ///
    /// * `file` - The file to read.
    /// * `shard_count` - The number of shards to create.
    ///
    /// # Returns
    ///
    /// A `Result` containing the constructed instance of [ShardedBuffer<CompleteIndex>]
    /// if successful, or an error if the file cannot be read or the index cannot be finalized.
    pub fn read_file(file: File, shard_count: usize) -> Result<Self> {
        let (mut index, remote) = InflightIndex::new(InflightIndexMode::File);
        remote.index_file(file.try_clone()?)?;
        assert!(index.try_finalize());

        Ok(Self {
            index: index.unwrap(),
            shards: ShardRepr::File {
                len: file.metadata()?.len(),
                file,
                shards: LruCache::new(NonZeroUsize::new(shard_count).unwrap()),
            },
        })
    }

    /// Reads a stream and returns a result.
    ///
    /// This function uses [InflightIndex] with [InflightIndexMode::Stream], which
    /// then it uses to completely index the stream. The index is then finalized and
    /// the resulting [CompleteIndex] is used to construct the [ShardedBuffer].
    ///
    /// # Arguments
    ///
    /// * `stream` - The stream to be read.
    ///
    /// # Returns
    ///
    /// A `Result` containing the constructed instance of [ShardedBuffer<CompleteIndex>]
    /// if successful, or an error if the file cannot be read or the index cannot be finalized.
    pub fn read_stream(stream: Stream) -> Result<Self> {
        let (mut index, remote) = InflightIndex::new(InflightIndexMode::Stream);
        let (sx, rx) = std::sync::mpsc::channel();
        remote.index_stream(stream, sx)?;
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

impl ShardedBuffer<InflightIndex> {
    /// Reads a file and returns a `Result` containing the result of the operation.
    /// 
    /// This function uses [InflightIndex] with [InflightIndexMode::File], which
    /// then it uses to set off the indexing process in the background. While the
    /// indexing process is ongoing, the [ShardedBuffer] can be used to read the
    /// file. The content is safe to read, though it may not represent the complete
    /// picture until the indexing process is complete. Once the indexing process
    /// is complete, the [ShardedBuffer] can be used to read the file as normal.
    /// 
    /// # Arguments
    ///
    /// * `file` - The file to be read.
    /// * `shard_count` - The number of shards to be created.
    ///
    /// # Returns
    ///
    /// A `Result` containing an instance of [ShardedBuffer<InflightIndex>] if the
    /// file was successfully read, or an error if the operation failed.
    pub fn read_file(file: File, shard_count: usize) -> Result<Self> {
        let (index, indexer) = InflightIndex::new(InflightIndexMode::File);
        std::thread::spawn({
            let file = file.try_clone()?;
            move || indexer.index_file(file)
        });

        Ok(Self {
            index,
            shards: ShardRepr::File {
                len: file.metadata()?.len(),
                file,
                shards: LruCache::new(NonZeroUsize::new(shard_count).unwrap()),
            },
        })
    }

    /// Reads a file and returns a `Result` containing the result of the operation.
    /// 
    /// This function uses [InflightIndex] with [InflightIndexMode::Stream], which
    /// then it uses to set off the indexing process in the background. While the
    /// indexing process is ongoing, the [ShardedBuffer] can be used to read the
    /// file. The content is safe to read, though it may not represent the complete
    /// picture until the indexing process is complete. Once the indexing process
    /// is complete, the [ShardedBuffer] can be used to read the file as normal.
    /// 
    /// # Arguments
    ///
    /// * `file` - The file to be read.
    /// * `shard_count` - The number of shards to be created.
    ///
    /// # Returns
    ///
    /// An instance of [ShardedBuffer<InflightIndex>].
    pub fn read_stream(stream: Stream) -> Self {
        let (index, indexer) = InflightIndex::new(InflightIndexMode::Stream);
        let (sx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || indexer.index_stream(stream, sx));

        Self {
            index,
            shards: ShardRepr::Stream {
                pending_shards: Some(rx),
                shards: Vec::new(),
            },
        }
    }

    /// Attempt to finalize the inner [InflightIndex].
    /// 
    /// See [`InflightIndex::try_finalize()`] for more information.
    pub fn try_finalize(&mut self) -> bool {
        self.index.try_finalize()
    }

    /// Report the progress of the inner [InflightIndex].
    pub fn progress(&self) -> InflightIndexProgress {
        self.index.progress()
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

    /// Return the index of this [ShardedBuffer].
    pub fn index(&self) -> &Idx {
        &self.index
    }

    /// Retrieves a line of text from the buffer based on the given line number.
    /// 
    /// # Arguments
    /// 
    /// * `line_number` - The line number to retrieve.
    /// 
    /// # Panics
    /// 
    /// This function will panic if the `line_number` is greater than the total number
    /// of lines in the buffer's index.
    /// 
    /// # Returns
    /// 
    /// The line of text as a [ShardStr] object.
    pub fn get_line(&mut self, line_number: usize) -> ShardStr {
        assert!(line_number <= self.line_count());

        let data_start = self.index.data_of_line(line_number).unwrap();
        let data_end = self.index.data_of_line(line_number + 1).unwrap();
        let shard_start = (data_start / crate::SHARD_SIZE) as usize;
        let shard_end = (data_end / crate::SHARD_SIZE) as usize;

        if shard_start == shard_end {
            // The data is in a single shard
            let shard = self.shards.fetch(shard_start as usize);
            let (start, end) = shard.translate_inner_data_range(data_start, data_end);
            shard.get_shard_line(start, end)
        } else {
            debug_assert!(shard_start < shard_end);
            // The data may cross several shards, so we must piece together
            // the data from across the shards.
            let mut buf = Vec::with_capacity((data_end - data_start) as usize);

            let shard_first = self.shards.fetch(shard_start as usize);
            let shard_last = self.shards.fetch(shard_end as usize);
            let (start, end) = (
                shard_first.translate_inner_data_index(data_start),
                shard_last.translate_inner_data_index(data_end),
            );
            buf.extend_from_slice(&shard_first[start as usize..]);
            for shard_id in shard_start + 1..shard_end {
                buf.extend_from_slice(&self.shards.fetch(shard_id));
            }
            buf.extend_from_slice(&shard_last[..end as usize]);

            let buf = String::from_utf8_lossy(&buf).into_owned();
            ShardStr::new_owned(buf)
        }
    }
}

impl<Idx> ShardedBuffer<Idx>
where
    Idx: BufferIndex + Clone,
{
    pub fn multibuffer_iter(&self) -> Result<MultibufferIterator<Idx>> {
        match &self.shards {
            ShardRepr::File { file, len, .. } => Ok(MultibufferIterator::new(
                self.index.clone(),
                0..self.index.line_count(),
                ShardRepr::File {
                    file: file.try_clone()?,
                    len: *len,
                    shards: LruCache::new(NonZeroUsize::new(2).unwrap()),
                },
            )),
            ShardRepr::Stream { shards, .. } => Ok(MultibufferIterator::new(
                self.index.clone(),
                0..self.index.line_count(),
                ShardRepr::Stream {
                    pending_shards: None,
                    shards: shards.clone(),
                },
            )),
        }
    }
}

pub struct MultibufferIterator<Idx> {
    index: Idx,
    shards: ShardRepr,
    line_range: Range<usize>,
    // Intermediate buffer for the iterator to borrow from
    // for the case where the line crosses multiple shards
    imm_buff: Vec<u8>,
    // Intermediate shard storage for the buffer to borrow from
    // for when the buffer lies within a single shard
    imm_shard: Option<Arc<Shard>>,
}

impl<Idx> MultibufferIterator<Idx>
where
    Idx: BufferIndex,
{
    fn new(index: Idx, line_range: Range<usize>, shards: ShardRepr) -> Self {
        Self {
            line_range,
            index,
            shards,
            imm_buff: Vec::new(),
            imm_shard: None,
        }
    }

    /// Get the next buffer from the [MultibufferIterator].
    ///
    /// This function retrieves the next buffer from the `MultibufferIterator` and returns it as an `Option`.
    /// If there are no more buffers available, it returns `None`.
    ///
    /// # Returns
    ///
    /// - `Some((&Idx, u64, &[u8]))`: A tuple containing the index, starting data
    ///                               position, and a slice of the buffer data.
    /// - `None`: If there are no more buffers available.
    pub fn next(&mut self) -> Option<(&Idx, u64, &[u8])> {
        if self.line_range.is_empty() {
            return None;
        }

        let curr_line = self.line_range.start;
        let curr_line_data_start = self.index.data_of_line(curr_line).unwrap();
        let curr_line_data_end = self.index.data_of_line(curr_line + 1).unwrap();

        let curr_line_shard_start = (curr_line_data_start / crate::SHARD_SIZE) as usize;
        let curr_line_shard_end = (curr_line_data_end / crate::SHARD_SIZE) as usize;

        if curr_line_shard_end != curr_line_shard_start {
            self.imm_buff.clear();
            self.imm_buff
                .reserve((curr_line_data_end - curr_line_data_start) as usize);

            let shard_first = self.shards.fetch(curr_line_shard_start);
            let shard_last = self.shards.fetch(curr_line_shard_end);
            let (start, end) = (
                shard_first.translate_inner_data_index(curr_line_data_start),
                shard_last.translate_inner_data_index(curr_line_data_end),
            );

            self.imm_buff
                .extend_from_slice(&shard_first[start as usize..]);
            for shard_id in curr_line_shard_start + 1..curr_line_shard_end {
                self.imm_buff
                    .extend_from_slice(&self.shards.fetch(shard_id));
            }
            self.imm_buff.extend_from_slice(&shard_last[..end as usize]);

            self.line_range.start += 1;
            return Some((&self.index, curr_line_data_start, &self.imm_buff));
        } else {
            let curr_shard_data_start = curr_line_shard_start as u64 * crate::SHARD_SIZE;
            let curr_shard_data_end = curr_shard_data_start + crate::SHARD_SIZE;

            let line_end = self
                .index
                .line_of_data(curr_shard_data_end)
                .unwrap_or_else(|| self.index.line_count());
            let line_end_data_start = self.index.data_of_line(line_end).unwrap();

            // this line should not cross multiple shards, else we would have caught in the first case
            let shard = self.shards.fetch(curr_line_shard_start);
            let (start, end) =
                shard.translate_inner_data_range(curr_line_data_start, line_end_data_start);
            assert!(line_end_data_start - curr_shard_data_start <= crate::SHARD_SIZE);
            assert!(end <= crate::SHARD_SIZE);

            self.line_range.start = line_end;
            let shard = self.imm_shard.insert(shard);

            // line must end at the boundary
            return Some((
                &self.index,
                curr_line_data_start,
                &shard[start as usize..end as usize],
            ));
        }
    }
}

#[cfg(test)]
mod test {
    use anyhow::Result;
    use std::{
        fs::File,
        io::{BufReader, Read},
    };

    use crate::{buf::ShardedBuffer, index::CompleteIndex};

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
                file_index.get_line(i).as_str(),
                stream_index.get_line(i).as_str()
            );
        }

        Ok(())
    }

    #[test]
    fn multi_buffer_consistency_1() -> Result<()> {
        multi_buffer_consistency_base(File::open("../../tests/test_10.log")?)
    }

    #[test]
    fn multi_buffer_consistency_2() -> Result<()> {
        multi_buffer_consistency_base(File::open("../../tests/test_50_long.log")?)
    }

    #[test]
    fn multi_buffer_consistency_3() -> Result<()> {
        multi_buffer_consistency_base(File::open("../../tests/test_5000000.log")?)
    }

    fn multi_buffer_consistency_base(file: File) -> Result<()> {
        let file_len = file.metadata()?.len();
        let mut reader = BufReader::new(file.try_clone()?);

        let file_buffer = ShardedBuffer::<CompleteIndex>::read_file(file, 25)?;
        let mut buffers = file_buffer.multibuffer_iter()?;

        let mut total_bytes = 0;
        let mut validate_buf = Vec::new();
        while let Some((_, start, buf)) = buffers.next() {
            // Validate that the specialized slice reader and normal sequential reads are consistent
            assert_eq!(start, total_bytes);
            total_bytes += buf.len() as u64;
            validate_buf.resize(buf.len(), 0);
            reader.read_exact(&mut validate_buf)?;
            assert_eq!(buf, validate_buf);
        }
        assert_eq!(total_bytes, file_len);

        Ok(())
    }
}
