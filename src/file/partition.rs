use std::{cmp::Ordering, ops::Range};

/// A data structure to partition a continuous spectrum
/// of numbers into indexed shards. This allows for
/// fast lookup
#[derive(Debug, PartialEq, Eq)]
pub struct RangePartition {
    inner: Vec<Range<usize>>,
}

impl RangePartition {
    /// Create a new range parition.
    pub const fn new() -> Self {
        Self { inner: Vec::new() }
    }

    /// List how many partitions there are.
    #[inline(always)]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Current (exclusive) end of the range of the last partition.
    pub fn curr_end(&self) -> usize {
        self.inner.last().map(|Range { end, .. }| *end).unwrap_or(0)
    }

    /// Insert a partition between the `self.curr_end()` and `end`.
    #[inline(always)]
    pub fn partition(&mut self, end: usize) {
        let start = self.curr_end();
        if start != end {
            self.inner.push(start..end);
        }
    }

    /// Lookup the partition range from the partition index.
    pub fn reverse_lookup(&self, value: usize) -> Option<Range<usize>> {
        self.inner.get(value).cloned()
    }

    /// Lookup the parition index from a number within some correspnoding partition range.
    pub fn lookup(&self, key: usize) -> Option<usize> {
        self.inner
            .binary_search_by(|probe| {
                if probe.start > key {
                    Ordering::Greater
                } else if probe.end <= key {
                    Ordering::Less
                } else {
                    Ordering::Equal
                }
            })
            .ok()
    }
}
