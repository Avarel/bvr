pub mod inflight;

use regex::bytes::Regex;

use crate::{buf::MultibufferIterator, cowvec::CowVec, index::BufferIndex, Result};

pub trait BufferSearch {
    fn find(&self, line_number: usize) -> bool;
}

#[derive(Clone)]
pub struct IncompleteSearch {
    inner: CompleteSearch,
}

impl IncompleteSearch {
    pub fn new() -> Self {
        Self {
            inner: CompleteSearch::empty(),
        }
    }

    fn search<Idx>(
        mut self,
        mut iter: MultibufferIterator<Idx>,
        regex: Regex,
    ) -> Result<CompleteSearch>
    where
        Idx: BufferIndex,
    {
        while let Some((idx, start, buf)) = iter.next() {
            for res in regex.find_iter(buf) {
                let match_start = res.start() as u64 + start;
                self.add_line(idx.line_of_data(match_start).unwrap())
            }
        }

        Ok(self.finish())
    }

    pub fn add_line(&mut self, line_number: usize) {
        self.inner.lines.push(line_number)
    }

    pub fn finish(self) -> CompleteSearch {
        self.inner
    }
}

#[derive(Clone)]
pub struct CompleteSearch {
    lines: CowVec<usize>,
}

impl CompleteSearch {
    pub fn empty() -> Self {
        Self {
            lines: CowVec::new(),
        }
    }
}

impl BufferSearch for CompleteSearch {
    fn find(&self, line_number: usize) -> bool {
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
}