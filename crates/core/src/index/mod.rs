//! Contains [IncompleteIndex], [CompleteIndex] and the trait [BufferIndex].
//! These abstractions allow for cheap clones of the append-only indices,
//! fast access to its information, and synchronous indexing operations.

pub mod inflight;

use crate::buf::segment::Segment;
use crate::cowvec::CowVec;
use crate::err::Result;
use std::fs::File;

/// The `BufferIndex` trait defines methods for working with line-based indexing of buffers.
pub trait BufferIndex {
    /// Returns the total number of lines that the `BufferIndex` can see.
    ///
    /// # Examples
    ///
    /// ```
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// use bvr_core::index::IncompleteIndex;
    /// use bvr_core::index::BufferIndex;
    ///
    /// let index = IncompleteIndex::new();
    /// let index = index.index_file(&std::fs::File::open("./Cargo.toml")?)?;
    /// dbg!(index.line_count());
    /// # Ok(())
    /// # }
    /// ```
    fn line_count(&self) -> usize;

    /// Returns the line number that corresponds to the start of the line
    /// that contains the given byte index.
    ///
    /// This is the inverse of `BufferIndex::data_of_line`.
    ///
    /// # Examples
    ///
    /// ```
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// use bvr_core::index::IncompleteIndex;
    /// use bvr_core::index::BufferIndex;
    ///
    /// let mut index = IncompleteIndex::new();
    /// index.push_line_data(10);
    /// index.finalize(100);
    /// let index = index.finish();
    /// // First line is 9 characters long, so 10 bytes with \n
    /// assert_eq!(index.line_of_data(0), Some(0));
    /// assert_eq!(index.line_of_data(4), Some(0));
    /// // Second line begins at byte 10
    /// assert_eq!(index.line_of_data(10), Some(1));
    /// assert_eq!(index.line_of_data(11), Some(1));
    /// // Out of bounds access is a None
    /// assert_eq!(index.line_of_data(1_000_000), None);
    /// # Ok(())
    /// # }
    /// ```
    fn line_of_data(&self, start: u64) -> Option<usize>;

    /// Returns the byte index that indicates start of the given line.
    /// Note that if `line_number` is equal to `BufferIndex::line_count`,
    /// then the result must be valid and the byte index must be the
    /// last index (exclusive) of the buffer.
    ///
    /// This is the inverse of `BufferIndex::line_of_data`.
    ///
    /// # Examples
    ///
    /// ```
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// use bvr_core::index::IncompleteIndex;
    /// use bvr_core::index::BufferIndex;
    ///
    /// let mut index = IncompleteIndex::new();
    /// index.push_line_data(10);
    /// index.finalize(100);
    /// let index = index.finish();
    /// // First line is 9 characters long, so 10 bytes with \n
    /// assert_eq!(index.data_of_line(0), Some(0));
    /// assert_eq!(index.data_of_line(1), Some(10));
    /// // Out of bounds access is a None
    /// assert_eq!(index.data_of_line(1_000_000), None);
    /// # Ok(())
    /// # }
    /// ```
    fn data_of_line(&self, line_number: usize) -> Option<u64>;
}

/// An index that can be built into a [CompleteIndex].
pub struct IncompleteIndex {
    inner: Index,
    finished: bool,
}

impl IncompleteIndex {
    /// Create a new [IncompleteIndex].
    ///
    /// This can be used to build a [CompleteIndex] by using the
    /// [`IncompleteIndex::index_file()`] method or manually
    /// using `push_line_data(u64)`, `finalize(u64)` and `finish()`.
    ///
    /// # Example
    /// ```
    /// use bvr_core::index::IncompleteIndex;
    /// let mut index = IncompleteIndex::new();
    /// ```
    pub fn new() -> Self {
        Self {
            inner: Index::empty(),
            finished: false,
        }
    }

    /// Index a [File] and return a [CompleteIndex].
    ///
    /// # Examples
    /// ```
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// use bvr_core::index::IncompleteIndex;
    /// use bvr_core::index::BufferIndex;
    ///
    /// let index = IncompleteIndex::new();
    /// let index = index.index_file(&std::fs::File::open("./Cargo.toml")?)?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn index_file(mut self, file: &File) -> Result<Index> {
        let len = file.metadata()?.len();
        let mut start = 0;

        while start < len {
            let end = (start + Segment::MAX_SIZE).min(len);

            let segment = Segment::map_file(start..end, file)?;

            for i in memchr::memchr_iter(b'\n', &segment) {
                let line_data = start + i as u64;
                self.push_line_data(line_data + 1);
            }

            start = end;
        }
        self.finalize(len);

        Ok(self.inner)
    }

    /// Push the starting byte of a new line into the index.
    pub fn push_line_data(&mut self, line_data: u64) {
        self.inner.line_index.push(line_data);
    }

    /// Finalize the index.
    pub fn finalize(&mut self, len: u64) {
        self.inner.line_index.push(len);
        self.finished = true;
    }

    /// Returns a [CompleteIndex].
    pub fn finish(self) -> Index {
        assert!(self.finished);
        self.inner
    }
}

impl Default for IncompleteIndex {
    fn default() -> Self {
        Self::new()
    }
}

/// A fixed and complete index.
#[derive(Clone)]
pub struct Index {
    /// Store the byte location of the start of the indexed line
    line_index: CowVec<u64>,
}

impl Index {
    /// Create an empty [CompleteIndex].
    fn empty() -> Self {
        Self {
            line_index: crate::cowvec![0],
        }
    }
}

impl BufferIndex for Index {
    fn line_count(&self) -> usize {
        self.line_index.len().saturating_sub(1)
    }

    fn data_of_line(&self, line_number: usize) -> Option<u64> {
        self.line_index.get(line_number).copied()
    }

    fn line_of_data(&self, key: u64) -> Option<usize> {
        // Safety: this code was pulled from Vec::binary_search_by
        let mut size = self.line_count();
        let mut left = 0;
        let mut right = size;
        while left < right {
            let mid = left + size / 2;

            // mid must be less than size, which is self.line_index.len() - 1
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
