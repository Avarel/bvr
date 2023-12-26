use crate::{
    buf::ContiguousSegmentIterator,
    cowvec::{CowVec, CowVecWriter},
    Result, SegBuffer,
};
use regex::bytes::Regex;
use std::sync::{atomic::AtomicBool, Arc};

pub struct LineMatchRemote {
    buf: CowVecWriter<usize>,
    complete: Arc<AtomicBool>,
}

impl LineMatchRemote {
    /// Index a file and load the data into the associated [InflightIndex].
    pub fn search(mut self, mut iter: ContiguousSegmentIterator, regex: Regex) -> Result<()> {
        while let Some((idx, start, buf)) = iter.next_buf() {
            for res in regex.find_iter(buf) {
                let match_start = res.start() as u64 + start;

                let line_number = idx.line_of_data(match_start).unwrap();

                if self.buf.last() == Some(&line_number) {
                    continue;
                } else if let Some(&last) = self.buf.last() {
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
        match slice.as_slice() {
            &[first, .., last] => {
                if (first..=last).contains(&line_number) {
                    return slice.binary_search(&line_number).is_ok();
                }
            }
            &[item] => return item == line_number,
            _ => (),
        }
        false
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }
}

#[derive(Clone)]
pub struct LineMatches {
    buf: CowVec<usize>,
    complete: Arc<AtomicBool>,
}

impl LineMatches {
    #[inline]
    pub fn new() -> (Self, LineMatchRemote) {
        let (buf, writer) = CowVec::new();
        let complete = Arc::new(AtomicBool::new(false));
        (
            Self {
                buf,
                complete: complete.clone(),
            },
            LineMatchRemote {
                buf: writer,
                complete,
            },
        )
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
        let (search, remote) = LineMatches::new();
        std::thread::spawn({
            let iter = buf.segment_iter()?;
            move || remote.search(iter, regex)
        });
        Ok(search)
    }
}
