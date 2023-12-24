use std::sync::Arc;

use regex::bytes::Regex;

use crate::buf::ContiguousSegmentIterator;
use crate::cowvec::CowVec;
use crate::inflight_tool::{Inflight, InflightImpl, Inflightable};
use crate::SegBuffer;
use crate::{index::BufferIndex, Result};

pub struct InflightMatchRemote(Arc<InflightImpl<CowVec<usize>>>);

impl InflightMatchRemote {
    /// Index a file and load the data into the associated [InflightIndex].
    pub fn search<Idx>(self, mut iter: ContiguousSegmentIterator<Idx>, regex: Regex) -> Result<()>
    where
        Idx: BufferIndex,
    {
        while let Some((idx, start, buf)) = iter.next_buf() {
            if Arc::strong_count(&self.0) <= 1 {
                break;
            }

            self.0.write(|inner| {
                for res in regex.find_iter(buf) {
                    let match_start = res.start() as u64 + start;

                    let line_number = idx.line_of_data(match_start).unwrap();
                    
                    if inner.last() == Some(&line_number) {
                        continue;
                    } else if let Some(last) = inner.last() {
                        assert!(line_number > *last);
                    }

                    inner.push(line_number)
                }
            });
        }
        self.0.mark_complete();
        Ok(())
    }
}

impl InflightMatches {
    pub fn is_complete(&self) -> bool {
        self.0.is_complete()
    }

    pub fn get(&self, idx: usize) -> Option<usize> {
        match &self.0 {
            Inflight::Incomplete(inner) => inner.read(|index| index.get(idx)),
            Inflight::Complete(index) => index.get(idx)
        }
    }

    pub fn has_line(&self, line_number: usize) -> bool {
        match &self.0 {
            Inflight::Incomplete(inner) => {
                inner.read(|index| Self::sorted_binary_search(index.as_slice(), line_number))
            }
            Inflight::Complete(index) => Self::sorted_binary_search(index.as_slice(), line_number),
        }
    }

    pub fn len(&self) -> usize {
        match &self.0 {
            Inflight::Incomplete(inner) => inner.read(|index| index.len()),
            Inflight::Complete(index) => index.len(),
        }
    }
}

impl<T> Inflightable for CowVec<T>
where
    T: Copy,
{
    type Incomplete = CowVec<T>;

    fn finish(inner: Self::Incomplete) -> Self {
        inner
    }

    fn snapshot(inner: &Self::Incomplete) -> Self {
        inner.clone()
    }
}

#[derive(Clone)]
pub struct InflightMatches(Inflight<CowVec<usize>>);

impl InflightMatches {
    pub fn new() -> (Self, InflightMatchRemote) {
        let inner = Arc::new(InflightImpl::<CowVec<usize>>::new());
        (Self(Inflight::Incomplete(inner.clone())), InflightMatchRemote(inner))
    }
    
    pub fn complete(inner: CowVec<usize>) -> Self {
        Self(Inflight::Complete(inner))
    }

    pub fn empty() -> Self {
        Self::complete(CowVec::new())
    }

    /// Searches for a regular expression pattern in a segmented buffer.
    ///
    /// # Returns
    ///
    /// Returns a `Result` containing the `InflightSearch` object
    /// if the internal iterator creation was successful, and an error otherwise.
    pub fn search<Idx>(buf: &SegBuffer<Idx>, regex: Regex) -> Result<Self>
    where
        Idx: BufferIndex + Clone + Send + 'static,
    {
        let (search, remote) = InflightMatches::new();
        std::thread::spawn({
            let iter = buf.segment_iter()?;
            move || remote.search(iter, regex)
        });
        Ok(search)
    }

    // TODO: generalize this
    fn sorted_binary_search(slice: &[usize], line_number: usize) -> bool {
        if let &[first, .., last] = slice {
            if (first..=last).contains(&line_number) {
                return slice.binary_search(&line_number).is_ok();
            }
        } else if let &[item] = slice {
            return item == line_number;
        }
        false
    }

    pub fn try_finalize(&mut self) -> bool {
        self.0.try_finalize()
    }
}