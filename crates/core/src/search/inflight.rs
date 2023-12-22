use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use regex::bytes::Regex;

use crate::buf::ContiguousSegmentIterator;
use crate::SegBuffer;
use crate::{index::BufferIndex, Result};

use super::{BufferSearch, CompleteSearch, IncompleteSearch};

#[doc(hidden)]
pub struct InflightSearchImpl {
    inner: std::sync::Mutex<IncompleteSearch>,
    cache: std::sync::Mutex<Option<CompleteSearch>>,
    progress: AtomicU64,
}

impl InflightSearchImpl {
    fn new() -> Arc<Self> {
        Arc::new(InflightSearchImpl {
            inner: std::sync::Mutex::new(IncompleteSearch::new()),
            cache: std::sync::Mutex::new(None),
            progress: AtomicU64::new(0),
        })
    }

    fn search<Idx>(
        self: Arc<Self>,
        mut iter: ContiguousSegmentIterator<Idx>,
        regex: Regex,
    ) -> Result<()>
    where
        Idx: BufferIndex,
    {
        assert!(Arc::strong_count(&self) >= 2);

        let start_range = iter.remaining_range();

        while let Some((idx, start, buf)) = iter.next_buf() {
            let mut lock = self.inner.lock().unwrap();
            for res in regex.find_iter(buf) {
                let match_start = res.start() as u64 + start;
                lock.add_line(idx.line_of_data(match_start).unwrap())
            }
            let start = iter.remaining_range().start;

            let progress =
                (start - start_range.start) as f64 / (start_range.end - start_range.start) as f64;
            self.progress.store(
                (progress * 100.0) as u64,
                std::sync::atomic::Ordering::Relaxed,
            );
        }

        Ok(())
    }

    fn read<F, T>(&self, cb: F) -> T
    where
        F: FnOnce(&CompleteSearch) -> T,
    {
        match self.inner.try_lock() {
            Ok(index) => {
                let clone = index.inner.clone();
                let val = cb(&clone);
                *self.cache.lock().unwrap() = Some(clone);
                val
            }
            Err(_) => {
                let lock = self.cache.lock().unwrap();
                if let Some(v) = lock.as_ref() {
                    return cb(v);
                }
                drop(lock);

                let clone = self.inner.lock().unwrap().inner.clone();
                let val = cb(&clone);
                *self.cache.lock().unwrap() = Some(clone);
                val
            }
        }
    }
}

pub struct InflightSearchRemote(Arc<InflightSearchImpl>);

impl InflightSearchRemote {
    /// Index a file and load the data into the associated [InflightIndex].
    pub fn search<Idx>(self, iter: ContiguousSegmentIterator<Idx>, regex: Regex) -> Result<()>
    where
        Idx: BufferIndex,
    {
        self.0.search(iter, regex)
    }
}

/// Progress report by [InflightSearch]'s `progress()` method.
pub enum InflightSearchProgress {
    Done,
    Partial(f64),
}

#[derive(Clone)]
pub enum InflightSearch {
    /// The indexing process is incomplete. The process must be started using
    /// the associated [InflightSearchRemote]. Accesses to the data inside
    /// are relatively cheap, with atomic copies of the ref-counted pointers
    /// to the internal buffers.
    Incomplete(#[doc(hidden)] Arc<InflightSearchImpl>),
    /// The indexing process is finalized, and the internal representation is
    /// replaced with a [CompleteSearch]. This can be obtained through
    /// [`Self::try_finalize()`].
    Complete(CompleteSearch),
}

impl InflightSearch {
    /// This function creates a new instance of [InflightSearch] and its associated [InflightSearchRemote].
    /// The [InflightSearch] is responsible for managing the state of an ongoing search operation,
    /// while the [InflightSearchRemote] provides a remote interface for starting off the search operation.
    ///
    /// # Examples
    ///
    /// ```
    /// use bvr_core::search::inflight::InflightSearch;
    ///
    /// let inflight_search = InflightSearch::new();
    /// ```
    pub fn new() -> (Self, InflightSearchRemote) {
        let inner = InflightSearchImpl::new();
        (Self::Incomplete(inner.clone()), InflightSearchRemote(inner))
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
        let (search, remote) = InflightSearch::new();
        std::thread::spawn({
            let iter = buf.segment_iter()?;
            move || remote.search(iter, regex)
        });
        Ok(search)
    }

    pub fn try_finalize(&mut self) -> bool {
        match self {
            Self::Incomplete(inner) => {
                match Arc::try_unwrap(std::mem::replace(
                    inner,
                    InflightSearchImpl::new(),
                )) {
                    Ok(unwrapped) => {
                        *self = Self::Complete(unwrapped.inner.into_inner().unwrap().finish());
                        true
                    },
                    Err(old_inner) => {
                        *self = Self::Incomplete(old_inner);
                        false
                    }
                }
            }
            Self::Complete(_) => true,
        }
    }

    pub fn progress(&self) -> InflightSearchProgress {
        match self {
            InflightSearch::Incomplete(inner) => {
                let progress =
                    inner.progress.load(std::sync::atomic::Ordering::Relaxed) as f64 / 100.0;
                InflightSearchProgress::Partial(progress)
            }
            InflightSearch::Complete(_) => InflightSearchProgress::Done,
        }
    }
}

impl BufferSearch for InflightSearch {
    fn get(&self, idx: usize) -> Option<usize> {
        match self {
            InflightSearch::Incomplete(inner) => inner.read(|index| index.get(idx)),
            InflightSearch::Complete(index) => index.get(idx),
        }
    }

    fn has_line(&self, line_number: usize) -> bool {
        match self {
            InflightSearch::Incomplete(inner) => inner.read(|index| index.has_line(line_number)),
            InflightSearch::Complete(index) => index.has_line(line_number),
        }
    }

    fn len(&self) -> usize {
        match self {
            InflightSearch::Incomplete(inner) => inner.read(|index| index.len()),
            InflightSearch::Complete(index) => index.len(),
        }
    }
}
