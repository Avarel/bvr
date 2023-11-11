pub mod inflight;

use crate::cowvec::CowVec;
use anyhow::Result;
use std::fs::File;

pub trait BufferIndex {
    /// Returns the total number of lines that the [BufferIndex] can see.
    ///
    /// # Examples
    ///
    /// ```
    /// let index = IncompleteIndex::new();
    /// let index = index.index(&std::fs::File::open("./Cargo.toml")?)?;
    /// dbg!(index.line_count());   
    /// ```
    fn line_count(&self) -> usize;

    /// Returns the line number that corresponds to the start of the line
    /// that contains the given byte index.
    /// 
    /// This is the inverse of `BufferIndex::data_of_line`.
    fn line_of_data(&self, start: u64) -> Option<usize>;

    /// Returns the byte index that indicates start of the given line.
    /// Note that if `line_number` is equal to `BufferIndex::line_count`,
    /// then the result must be valid and the byte index must be the
    /// last index (exclusive) of the buffer.
    /// 
    /// This is the inverse of `BufferIndex::line_of_data`.
    fn data_of_line(&self, line_number: usize) -> Option<u64>;
}

pub struct IncompleteIndex {
    inner: CompleteIndex,
    finished: bool,
}

impl IncompleteIndex {
    /// Create a new [IncompleteIndex]. This can be used to build a
    /// [CompleteIndex] by using the `index(&File)` method or manually
    /// using `push_line_data`.
    pub fn new() -> Self {
        Self {
            inner: CompleteIndex::empty(),
            finished: false,
        }
    }

    /// Index a [File] and return a [CompleteIndex].
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

    /// Push the starting byte of a new line into the index.
    fn push_line_data(&mut self, line_data: u64) {
        self.inner.line_index.push(line_data);
    }

    /// Finalize the index.
    fn finalize(&mut self, len: u64) {
        self.inner.line_index.push(len);
        self.finished = true;
    }

    /// Returns a [CompleteIndex].
    fn finish(self) -> CompleteIndex {
        assert!(self.finished);
        self.inner
    }
}

/// A fixed and complete index.
#[derive(Clone)]
pub struct CompleteIndex {
    /// Store the byte location of the start of the indexed line
    line_index: CowVec<u64>,
}

impl CompleteIndex {
    /// Create an empty [CompleteIndex].
    fn empty() -> Self {
        Self {
            line_index: CowVec::new_one_elem(0),
        }
    }
}

impl BufferIndex for CompleteIndex {
    fn line_count(&self) -> usize {
        self.line_index.len().saturating_sub(1)
    }

    fn data_of_line(&self, line_number: usize) -> Option<u64> {
        self.line_index.get(line_number).copied()
    }

    fn line_of_data(&self, key: u64) -> Option<usize> {
        // Safety: this code was pulled from Vec::binary_search_by
        let mut size = self.line_index.len();
        let mut left = 0;
        let mut right = size;
        while left < right {
            let mid = left + size / 2;

            let start = unsafe { *self.line_index.get_unchecked(mid) };
            let end = unsafe { *self.line_index.get_unchecked(mid + 1) };

            if end <= key {
                left = mid + 1;
            } else if start > key {
                right = mid;
            } else {
                return Some(mid);
            }

            size = right - left;
        }

        None
    }
}
