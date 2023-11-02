use std::ops::Range;

/// A data structure to partition a continuous spectrum
/// of numbers into indexed shards. This allows for
/// fast lookup.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct RangePartition {
    inner: Vec<usize>,
}

impl RangePartition {
    /// Create a new range parition.
    pub fn new() -> Self {
        Self { inner: vec![0] }
    }

    /// List how many partitions there are.
    #[inline(always)]
    pub fn len(&self) -> usize {
        self.inner.len().saturating_sub(1)
    }

    /// Current (exclusive) end of the range of the last partition.
    pub fn curr_end(&self) -> usize {
        self.inner.last().copied().unwrap_or(0)
    }

    /// Insert a partition between the `self.curr_end()` and `end`.
    #[inline(always)]
    pub fn partition(&mut self, end: usize) {
        let start = self.curr_end();
        if start < end {
            self.inner.push(end);
        }
    }

    /// Lookup the partition range from the partition index.
    pub fn reverse_lookup(&self, value: usize) -> Option<Range<usize>> {
        if value < self.len() {
            // Safety: we did our range checks
            unsafe { Some(*self.inner.get_unchecked(value)..*self.inner.get_unchecked(value + 1)) }
        } else {
            None
        }
    }

    /// Lookup the parition index from a number within some correspnoding partition range.
    pub fn lookup(&self, key: usize) -> Option<usize> {
        // Safety: this code was pulled from Vec::binary_search_by
        let mut size = self.len();
        let mut left = 0;
        let mut right = size;
        while left < right {
            let mid = left + size / 2;

            let start = unsafe { *self.inner.get_unchecked(mid) };
            let end = unsafe { *self.inner.get_unchecked(mid + 1) };

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
