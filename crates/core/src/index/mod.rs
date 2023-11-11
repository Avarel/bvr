pub mod inflight;

use crate::Mmappable;
use std::fs::File;
use std::ops::Range;

use anyhow::Result;
use tokio::sync::mpsc::Receiver;

// use partition::RangePartition;

use crate::cowvec::CowVec;

struct IndexingTask {
    sx: tokio::sync::mpsc::Sender<u64>,
    data: memmap2::Mmap,
    start: u64,
}

impl IndexingTask {
    const HEURISTIC_LINES_PER_MB: usize = 1 << 13;

    fn get_data<T: Mmappable>(file: &T, start: u64, end: u64) -> Result<memmap2::Mmap> {
        let data = unsafe {
            memmap2::MmapOptions::new()
                .offset(start)
                .len((end - start) as usize)
                .map(file)?
        };
        #[cfg(unix)]
        data.advise(memmap2::Advice::Sequential)?;
        Ok(data)
    }

    fn new<T: Mmappable>(file: &T, start: u64, end: u64) -> Result<(Self, Receiver<u64>)> {
        let data = Self::get_data(file, start, end)?;
        let (sx, rx) = tokio::sync::mpsc::channel(Self::HEURISTIC_LINES_PER_MB);
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
    fn start_of_line(&self, line_number: usize) -> u64;
    fn translate_data_from_line_range(&self, line_range: Range<usize>) -> Range<u64>;
}

pub struct IncompleteIndex {
    inner: CompleteIndex,
    finished: bool,
}

impl IncompleteIndex {
    pub fn new() -> Self {
        Self {
            inner: CompleteIndex::empty(),
            finished: false,
        }
    }

    pub fn index(mut self, file: &File) -> Result<CompleteIndex> {
        let len = file.metadata()?.len();
        let mut start = 0;

        while start < len {
            let end = (start + crate::INDEXING_VIEW_SIZE).min(len);

            let data = unsafe {
                memmap2::MmapOptions::new()
                    .offset(start)
                    .len((end - start) as usize)
                    .map(file)?
            };
            #[cfg(unix)]
            data.advise(memmap2::Advice::Sequential)?;

            for i in memchr::memchr_iter(b'\n', &data) {
                let line_data = start + i as u64;
                self.push_line_data(line_data + 1);
            }

            start = end;
        }
        self.finalize(len);

        Ok(self.inner)
    }

    fn push_line_data(&mut self, line_data: u64) {
        self.inner.line_index.push(line_data);
    }

    fn finalize(&mut self, len: u64) {
        self.inner.line_index.push(len);

        self.finished = true;
    }

    fn finish(self) -> CompleteIndex {
        assert!(self.finished);
        self.inner
    }
}

#[derive(Clone)]
pub struct CompleteIndex {
    /// Store the byte location of the start of the indexed line
    line_index: CowVec<u64>,
}

impl CompleteIndex {
    fn empty() -> Self {
        Self {
            line_index: CowVec::new_one_elem(0),
        }
    }
}

impl FileIndex for CompleteIndex {
    fn line_count(&self) -> usize {
        self.line_index.len().saturating_sub(1)
    }

    fn start_of_line(&self, line_number: usize) -> u64 {
        self.line_index[line_number]
    }

    fn translate_data_from_line_range(&self, line_range: Range<usize>) -> Range<u64> {
        self.start_of_line(line_range.start)..self.start_of_line(line_range.end)
    }
}
