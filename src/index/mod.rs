pub mod sync;
mod partition;
mod atomicvec;

use std::fs::File;
use std::ops::Range;
use std::os::fd::AsRawFd;

use anyhow::Result;
use tokio::sync::mpsc::Receiver;

use partition::RangePartition;

struct IndexingTask {
    sx: tokio::sync::mpsc::Sender<u64>,
    data: memmap2::Mmap,
    start: u64,
}

impl IndexingTask {
    const LINES_PER_MB: usize = 1 << 13;

    fn get_data<T: AsRawFd>(file: &T, start: u64, end: u64) -> Result<memmap2::Mmap> {
        let data = unsafe {
            memmap2::MmapOptions::new()
                .offset(start)
                .len((end - start) as usize)
                .map(file)?
        };
        data.advise(memmap2::Advice::Sequential)?;
        Ok(data)
    }

    fn new<T: AsRawFd>(file: &T, start: u64, end: u64) -> Result<(Self, Receiver<u64>)> {
        let data = Self::get_data(file, start, end)?;
        let (sx, rx) = tokio::sync::mpsc::channel(Self::LINES_PER_MB);
        Ok((Self { sx, data, start }, rx))
    }

    async fn worker_async(self) -> Result<()> {
        for i in memchr::memchr_iter(b'\n', &self.data) {
            self.sx.send(self.start + i as u64).await?;
        }

        Ok(())
    }
}

pub trait FileIndex {
    fn line_count(&self) -> usize;
    fn shard_count(&self) -> usize;
    fn start_of_line(&self, line_number: usize) -> u64;
    fn shard_of_line(&self, line_number: usize) -> Option<usize>;
    fn translate_data_from_line_range(&self, line_range: Range<usize>) -> Range<u64>;
    fn line_range_of_shard(&self, shard_id: usize) -> Option<Range<usize>>;
    fn data_range_of_shard(&self, shard_id: usize) -> Option<Range<u64>>;
}

#[derive(Debug)]
pub struct IncompleteIndex {
    inner: CompleteIndex,
    data_size: u64,
    finished: bool,
}

impl IncompleteIndex {
    /// Constant to determine how much data is stored in each shard.
    /// Note that the shard likely contains more than the threshold,
    /// this is just a cutoff.
    const SHARD_DATA_THRESHOLD: u64 = 1 << 20;

    /// How much data of the file should each indexing task handle?
    const INDEXING_VIEW_SIZE: u64 = 1 << 20;

    pub fn new() -> Self {
        Self {
            inner: CompleteIndex::empty(),
            data_size: 0,
            finished: false,
        }
    }

    pub fn index(mut self, file: &File) -> Result<CompleteIndex> {
        let len = file.metadata()?.len();
        let mut start = 0;

        while start < len {
            let end = (start + IncompleteIndex::INDEXING_VIEW_SIZE).min(len);

            let data = unsafe {
                memmap2::MmapOptions::new()
                    .offset(start)
                    .len((end - start) as usize)
                    .map(file)?
            };
            data.advise(memmap2::Advice::Sequential)?;

            for i in memchr::memchr_iter(b'\n', &data) {
                let line_data = start + i as u64;
                self.push_line_data(line_data);
            }

            start = end;
        }
        self.finalize(len);

        Ok(self.inner)
    }

    fn push_line_data(&mut self, line_data: u64) {
        let line_number = self.inner.line_index.len();
        self.inner.line_index.push(line_data);

        self.data_size += line_data;
        if self.data_size > Self::SHARD_DATA_THRESHOLD {
            self.inner.shard_partition.partition(line_number);
            self.data_size = 0;
        }
    }

    fn finalize(&mut self, len: u64) {
        self.inner.line_index.push(len);
        // In case the shard boundary did not end on the very last line we iterated through
        self.inner
            .shard_partition
            .partition(self.inner.line_index.len() - 1);

        self.finished = true;
    }

    fn finish(self) -> CompleteIndex {
        assert!(self.finished);
        self.inner
    }
}

#[derive(Debug, Clone)]
pub struct CompleteIndex {
    /// Store the byte location of the start of the indexed line
    line_index: Vec<u64>,
    /// Allow queries from line number in a line range to shard
    shard_partition: RangePartition,
}

impl CompleteIndex {
    fn empty() -> Self {
        Self {
            line_index: vec![0],
            shard_partition: RangePartition::new(),
        }
    }
}

impl FileIndex for CompleteIndex {
    fn line_count(&self) -> usize {
        self.line_index.len().saturating_sub(2)
    }

    fn shard_count(&self) -> usize {
        self.shard_partition.len()
    }

    fn start_of_line(&self, line_number: usize) -> u64 {
        self.line_index[line_number]
    }

    fn shard_of_line(&self, line_number: usize) -> Option<usize> {
        self.shard_partition.lookup(line_number)
    }

    fn translate_data_from_line_range(&self, line_range: Range<usize>) -> Range<u64> {
        self.start_of_line(line_range.start)..self.start_of_line(line_range.end)
    }

    fn line_range_of_shard(&self, shard_id: usize) -> Option<Range<usize>> {
        self.shard_partition.reverse_lookup(shard_id)
    }

    fn data_range_of_shard(&self, shard_id: usize) -> Option<Range<u64>> {
        Some(self.translate_data_from_line_range(self.line_range_of_shard(shard_id)?))
    }
}
