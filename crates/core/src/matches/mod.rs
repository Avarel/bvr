pub mod inflight;

use regex::bytes::Regex;

use crate::{buf::ContiguousSegmentIterator, cowvec::CowVec, index::BufferIndex, Result};

pub trait BufferMatches {
    fn get(&self, index: usize) -> Option<usize>;
    fn has_line(&self, line_number: usize) -> bool;
    fn len(&self) -> usize;
    fn is_complete(&self) -> bool;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[derive(Clone)]
pub struct IncompleteMatches {
    inner: Matches,
}

impl IncompleteMatches {
    /// Create a new [IncompleteSearch].
    pub fn new() -> Self {
        Self {
            inner: Matches::empty(),
        }
    }

    /// Search for a regex in a buffer.
    #[allow(dead_code)]
    fn search<Idx>(
        mut self,
        mut iter: ContiguousSegmentIterator<Idx>,
        regex: Regex,
    ) -> Result<Matches>
    where
        Idx: BufferIndex,
    {
        while let Some((idx, start, buf)) = iter.next_buf() {
            for res in regex.find_iter(buf) {
                let match_start = res.start() as u64 + start;
                self.add_line(idx.line_of_data(match_start).unwrap())
            }
        }

        Ok(self.finish())
    }

    pub fn add_line(&mut self, line_number: usize) {
        if self.inner.lines.last() == Some(&line_number) {
            return;
        }
        self.inner.lines.push(line_number)
    }

    #[must_use]
    pub fn finish(self) -> Matches {
        self.inner
    }
}

impl Default for IncompleteMatches {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone)]
pub struct Matches {
    lines: CowVec<usize>,
}

impl Matches {
    pub fn empty() -> Self {
        Self {
            lines: CowVec::new(),
        }
    }
}

impl BufferMatches for Matches {
    fn get(&self, index: usize) -> Option<usize> {
        self.lines.get(index).copied()
    }

    fn is_complete(&self) -> bool {
        true
    }

    fn has_line(&self, line_number: usize) -> bool {
        let slice = self.lines.as_slice();
        if let &[first, .., last] = slice {
            if (first..=last).contains(&line_number) {
                return slice.binary_search(&line_number).is_ok();
            }
        } else if let &[item] = slice {
            return item == line_number;
        }
        false
    }

    fn len(&self) -> usize {
        self.lines.len()
    }
}
