pub mod composite;

use crate::buf::ContiguousSegmentIterator;
use crate::cowvec::{CowVec, CowVecSnapshot, CowVecWriter};
use crate::{LineIndex, Result};
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
            if !self.has_readers() {
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

    pub fn has_readers(&self) -> bool {
        Arc::strong_count(&self.completed) > 1
    }
}

impl Drop for LineMatchRemote {
    fn drop(&mut self) {
        self.completed
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

#[derive(Clone)]
pub enum LineSet {
    All {
        buf: LineIndex,
    },
    Subset {
        buf: Arc<CowVec<usize>>,
        completed: Arc<AtomicBool>,
        // Optimization field for composite filters
        // Minimum length of all filters combined
        min_len: usize,
    },
}

impl LineSet {
    #[inline]
    pub fn empty() -> Self {
        Self::Subset {
            buf: Arc::new(CowVec::empty()),
            completed: Arc::new(AtomicBool::new(true)),
            min_len: 0,
        }
    }

    pub fn all(buf: LineIndex) -> Self {
        Self::All { buf }
    }

    pub fn is_all(&self) -> bool {
        matches!(self, Self::All { .. })
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
        Self::Subset {
            buf,
            completed: complete,
            min_len: 0,
        }
    }

    #[inline]
    pub fn compose(
        mut filters: Vec<Self>,
        complete: bool,
        strategy: CompositeStrategy,
    ) -> Result<Self> {
        match filters.len() {
            0 => Ok(Self::empty()),
            1 => Ok(filters.remove(0)),
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
                Ok(Self::Subset {
                    buf,
                    completed,
                    min_len,
                })
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn into_inner(self) -> CowVec<usize> {
        match self {
            Self::All { .. } => unimplemented!(),
            Self::Subset { buf, .. } => Arc::try_unwrap(buf).unwrap(),
        }
    }

    #[inline]
    pub fn is_complete(&self) -> bool {
        match self {
            LineSet::All { buf } => buf.is_complete(),
            LineSet::Subset { completed, .. } => {
                completed.load(std::sync::atomic::Ordering::Relaxed)
            }
        }
    }

    pub fn get(&self, idx: usize) -> Option<usize> {
        match self {
            LineSet::All { buf } => {
                if idx < buf.line_count() {
                    Some(idx)
                } else {
                    None
                }
            }
            LineSet::Subset { buf, .. } => buf.get(idx),
        }
    }

    pub fn find(&self, line_number: usize) -> Option<usize> {
        match self {
            LineSet::All { buf } => {
                if line_number < buf.line_count() {
                    Some(line_number)
                } else {
                    None
                }
            }
            LineSet::Subset { buf, .. } => {
                let slice = buf.snapshot();
                match *slice.as_slice() {
                    [first, .., last] if (first..=last).contains(&line_number) => {
                        slice.binary_search(&line_number).ok()
                    }
                    [item] if item == line_number => Some(0),
                    _ => None,
                }
            }
        }
    }

    pub fn has_line(&self, line_number: usize) -> bool {
        self.find(line_number).is_some()
    }

    pub fn nearest_forward(&self, line_number: usize) -> Option<usize> {
        match self {
            LineSet::All { buf } => {
                if line_number < buf.line_count() {
                    Some((line_number + 1).min(buf.line_count()))
                } else {
                    None
                }
            }
            LineSet::Subset { buf, .. } => {
                let snap = buf.snapshot();
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
        }
    }

    pub fn nearest_backward(&self, line_number: usize) -> Option<usize> {
        match self {
            LineSet::All { .. } => line_number.checked_sub(1),
            LineSet::Subset { buf, .. } => {
                let snap = buf.snapshot();
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
        }
    }

    pub fn len(&self) -> usize {
        match self {
            LineSet::All { buf } => buf.line_count(),
            LineSet::Subset { buf, min_len, .. } => buf.len().max(*min_len),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub(crate) fn snapshot(&self) -> Option<CowVecSnapshot<usize>> {
        match self {
            LineSet::All { .. } => None,
            LineSet::Subset { buf, .. } => Some(buf.snapshot()),
        }
    }
}

impl From<Vec<usize>> for LineSet {
    fn from(vec: Vec<usize>) -> Self {
        Self::Subset {
            min_len: vec.len(),
            buf: Arc::new(CowVec::from(vec)),
            completed: Arc::new(AtomicBool::new(true)),
        }
    }
}
