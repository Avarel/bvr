use crate::{
    buf::ContiguousSegmentIterator,
    cowvec::{
        inflight::{InflightVec, InflightVecWriter},
        CowVec,
    },
    Result, SegBuffer,
};
use regex::bytes::Regex;
use std::sync::Arc;

pub struct LineMatchRemote(Arc<InflightVecWriter<usize>>);

impl LineMatchRemote {
    /// Index a file and load the data into the associated [InflightIndex].
    pub fn search(self, mut iter: ContiguousSegmentIterator, regex: Regex) -> Result<()> {
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
                    } else if let Some(&last) = inner.last() {
                        debug_assert!(line_number > last);
                    }

                    inner.push(line_number)
                }
            });
        }
        self.0.mark_complete();
        Ok(())
    }
}

impl LineMatches {
    #[inline]
    pub fn is_complete(&self) -> bool {
        self.0.is_complete()
    }

    pub fn get(&self, idx: usize) -> Option<usize> {
        self.0.read(|index| index.get(idx))
    }

    pub fn has_line(&self, line_number: usize) -> bool {
        self.0.read(|index| {
            let slice = index.as_slice();
            if let &[first, .., last] = slice {
                if (first..=last).contains(&line_number) {
                    return slice.binary_search(&line_number).is_ok();
                }
            } else if let &[item] = slice {
                return item == line_number;
            }
            false
        })
    }

    pub fn len(&self) -> usize {
        self.0.read(|index| index.len())
    }
}

#[derive(Clone)]
pub struct LineMatches(InflightVec<usize>);

impl LineMatches {
    #[inline]
    pub fn new() -> (Self, LineMatchRemote) {
        let inner = Arc::new(InflightVecWriter::<usize>::new());
        (
            Self(InflightVec::Incomplete(inner.clone())),
            LineMatchRemote(inner),
        )
    }

    #[inline]
    pub fn complete_from_vec(inner: Vec<usize>) -> Self {
        Self(InflightVec::Complete(CowVec::from(inner)))
    }

    #[inline]
    pub fn complete(inner: CowVec<usize>) -> Self {
        Self(InflightVec::Complete(inner))
    }

    #[inline]
    pub fn empty() -> Self {
        Self::complete(CowVec::new())
    }

    /// Searches for a regular expression pattern in a segmented buffer.
    ///
    /// # Returns
    ///
    /// Returns a `Result` containing the `InflightSearch` object
    /// if the internal iterator creation was successful, and an error otherwise.
    pub fn search(buf: &SegBuffer, regex: Regex) -> Result<Self> {
        let (search, remote) = LineMatches::new();
        std::thread::spawn({
            let iter = buf.segment_iter()?;
            move || remote.search(iter, regex)
        });
        Ok(search)
    }

    #[inline]
    pub fn try_finalize(&mut self) -> bool {
        self.0.try_finalize()
    }
}
