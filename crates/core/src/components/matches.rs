use crate::{
    buf::ContiguousSegmentIterator,
    cowvec::{CowVec, CowVecWriter},
    Result, SegBuffer,
};
use regex::bytes::Regex;
use std::sync::{atomic::AtomicBool, Arc};

struct LineMatchRemote {
    buf: CowVecWriter<usize>,
    complete: Arc<AtomicBool>,
}

impl LineMatchRemote {
    /// Index a file and load the data into the associated [InflightIndex].
    pub fn search(mut self, mut iter: ContiguousSegmentIterator, regex: Regex) -> Result<()> {
        while let Some((idx, start, buf)) = iter.next_buf() {
            for res in regex.find_iter(buf) {
                if !self.buf.has_readers() {
                    break;
                }

                let match_start = res.start() as u64 + start;

                let line_number = idx.line_of_data(match_start).unwrap();

                if let Some(&last) = self.buf.last() {
                    if last == line_number {
                        continue;
                    }
                    debug_assert!(line_number > last);
                }

                self.buf.push(line_number)
            }
        }
        self.mark_complete();
        Ok(())
    }

    fn mark_complete(&self) {
        self.complete
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

impl LineMatches {
    #[inline]
    pub fn is_complete(&self) -> bool {
        self.complete.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn get(&self, idx: usize) -> Option<usize> {
        self.buf.get(idx)
    }

    pub fn has_line(&self, line_number: usize) -> bool {
        let slice = self.buf.snapshot();
        match *slice.as_slice() {
            [first, .., last] => {
                if (first..=last).contains(&line_number) {
                    return slice.binary_search(&line_number).is_ok();
                }
            }
            [item] => return item == line_number,
            _ => (),
        }
        false
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }
}

#[derive(Clone)]
pub struct LineMatches {
    buf: CowVec<usize>,
    complete: Arc<AtomicBool>,
}

impl LineMatches {
    #[inline]
    pub fn new(iter: ContiguousSegmentIterator, regex: Regex) -> Self {
        let (buf, writer) = CowVec::new();
        let complete = Arc::new(AtomicBool::new(false));
        std::thread::spawn({
            let complete = complete.clone();
            move || {
                LineMatchRemote {
                    buf: writer,
                    complete,
                }
                .search(iter, regex)
            }
        });
        Self { buf, complete }
    }

    #[inline]
    pub fn complete_from_vec(inner: Vec<usize>) -> Self {
        Self {
            buf: CowVec::from(inner),
            complete: Arc::new(AtomicBool::new(true)),
        }
    }

    #[inline]
    pub fn empty() -> Self {
        Self {
            buf: CowVec::new().0,
            complete: Arc::new(AtomicBool::new(true)),
        }
    }

    /// Searches for a regular expression pattern in a segmented buffer.
    ///
    /// # Returns
    ///
    /// Returns a `Result` containing the `InflightSearch` object
    /// if the internal iterator creation was successful, and an error otherwise.
    pub fn search(buf: &SegBuffer, regex: Regex) -> Result<Self> {
        Ok(Self::new(buf.segment_iter()?, regex))
    }
}
