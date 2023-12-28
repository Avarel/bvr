pub mod composite;

use crate::{
    buf::ContiguousSegmentIterator,
    cowvec::{CowVec, CowVecSnapshot, CowVecWriter},
    Result,
};
use regex::bytes::Regex;
use std::sync::{atomic::AtomicBool, Arc};

pub use composite::CompositeStrategy;

struct LineMatchRemote {
    buf: CowVecWriter<usize>,
    completed: Arc<AtomicBool>,
}

impl LineMatchRemote {
    pub fn search(mut self, mut iter: ContiguousSegmentIterator, regex: Regex) -> Result<()> {
        while let Some((idx, start, buf)) = iter.next_buf() {
            if !self.buf.has_readers() {
                break;
            }

            let mut buf_start = 0;
            while let Some(res) = regex.find_at(buf, buf_start as usize) {
                let match_start = res.start() as u64 + start;
                let line_number = idx.line_of_data(match_start).unwrap();

                if let Some(&last) = self.buf.last() {
                    if last == line_number {
                        continue;
                    }
                    debug_assert!(line_number > last);
                }

                self.buf.push(line_number);

                buf_start = idx.data_of_line(line_number + 1).unwrap() - start;
            }
        }
        Ok(())
    }
}

impl Drop for LineMatchRemote {
    fn drop(&mut self) {
        self.completed
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

#[derive(Clone)]
pub struct LineMatches {
    buf: CowVec<usize>,
    completed: Arc<AtomicBool>,
    // Optimization field for composite filters
    // Minimum length of all filters combined
    min_len: usize,
}

impl LineMatches {
    #[inline]
    pub fn empty() -> Self {
        Self {
            buf: CowVec::empty(),
            completed: Arc::new(AtomicBool::new(true)),
            min_len: 0,
        }
    }

    #[inline]
    pub fn search(iter: ContiguousSegmentIterator, regex: Regex) -> Self {
        let (buf, writer) = CowVec::new();
        let complete = Arc::new(AtomicBool::new(false));
        std::thread::spawn({
            let complete = complete.clone();
            move || {
                LineMatchRemote {
                    buf: writer,
                    completed: complete,
                }
                .search(iter, regex)
            }
        });
        Self {
            buf,
            completed: complete,
            min_len: 0,
        }
    }

    #[inline]
    pub fn compose(
        filters: Vec<Self>,
        complete: bool,
        strategy: CompositeStrategy,
    ) -> Result<Self> {
        match filters.len() {
            0 => Ok(Self::empty()),
            1 => Ok(Self {
                buf: filters.into_iter().next().unwrap().into_inner(),
                completed: Arc::new(AtomicBool::new(true)),
                min_len: 0,
            }),
            _ => {
                let min_len = match strategy {
                    CompositeStrategy::Intersection => 0,
                    CompositeStrategy::Union => filters.iter().map(|f| f.len()).max().unwrap(),
                };
                let (buf, writer) = CowVec::new();
                let completed = Arc::new(AtomicBool::new(false));
                let task = {
                    let completed = completed.clone();
                    move || {
                        composite::LineCompositeRemote {
                            buf: writer,
                            completed,
                            strategy,
                        }
                        .compose(filters)
                    }
                };
                if complete {
                    task()?;
                } else {
                    std::thread::spawn(task);
                }
                Ok(Self {
                    buf,
                    completed,
                    min_len,
                })
            }
        }
    }

    pub(crate) fn into_inner(self) -> CowVec<usize> {
        self.buf
    }

    #[inline]
    pub fn is_complete(&self) -> bool {
        self.completed.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn get(&self, idx: usize) -> Option<usize> {
        self.buf.get(idx)
    }

    pub fn find(&self, line_number: usize) -> Option<usize> {
        let slice = self.buf.snapshot();
        match *slice.as_slice() {
            [first, .., last] if (first..=last).contains(&line_number) => {
                slice.binary_search(&line_number).ok()
            }
            [item] if item == line_number => Some(0),
            _ => None,
        }
    }

    pub fn has_line(&self, line_number: usize) -> bool {
        self.find(line_number).is_some()
    }

    pub fn nearest_forward(&self, line_number: usize) -> Option<usize> {
        let snap = self.buf.snapshot();
        let slice = snap.as_slice();
        match *slice {
            [_, ..] => Some(
                slice[match slice.binary_search(&line_number) {
                    Ok(i) => i.saturating_add(1),
                    Err(i) => i,
                }
                .min(slice.len() - 1)],
            ),
            [] => None,
        }
    }

    pub fn nearest_backward(&self, line_number: usize) -> Option<usize> {
        let snap = self.buf.snapshot();
        let slice = snap.as_slice();
        match *slice {
            [_, ..] => Some(
                slice[match slice.binary_search(&line_number) {
                    Ok(i) | Err(i) => i,
                }
                .saturating_sub(1)
                .min(slice.len() - 1)],
            ),
            [] => None,
        }
    }

    pub fn len(&self) -> usize {
        self.min_len.max(self.buf.len())
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    pub(crate) fn snapshot(&self) -> CowVecSnapshot<usize> {
        self.buf.snapshot()
    }
}

impl From<Vec<usize>> for LineMatches {
    fn from(vec: Vec<usize>) -> Self {
        Self {
            min_len: vec.len(),
            buf: CowVec::from(vec),
            completed: Arc::new(AtomicBool::new(true)),
        }
    }
}
