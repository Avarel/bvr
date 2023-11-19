use std::sync::Arc;

use regex::bytes::Regex;

use crate::buf::MultibufferIterator;
use crate::ShardedBuffer;
use crate::{index::BufferIndex, Result};

use super::{BufferSearch, CompleteSearch, IncompleteSearch};

#[doc(hidden)]
pub struct InflightSearchImpl {
    inner: std::sync::Mutex<IncompleteSearch>,
    cache: std::sync::Mutex<Option<CompleteSearch>>,
}

impl InflightSearchImpl {
    fn new() -> Arc<Self> {
        Arc::new(InflightSearchImpl {
            inner: std::sync::Mutex::new(IncompleteSearch::new()),
            cache: std::sync::Mutex::new(None),
        })
    }

    fn search<Idx>(self: Arc<Self>, mut iter: MultibufferIterator<Idx>, regex: Regex) -> Result<()>
    where
        Idx: BufferIndex,
    {
        assert_eq!(Arc::strong_count(&self), 2);

        while let Some((idx, start, buf)) = iter.next() {
            let mut lock = self.inner.lock().unwrap();
            for res in regex.find_iter(buf) {
                let match_start = res.start() as u64 + start;
                lock.add_line(idx.line_of_data(match_start).unwrap())
            }
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
                    return cb(&v);
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
    pub fn search<Idx>(self, iter: MultibufferIterator<Idx>, regex: Regex) -> Result<()>
    where
        Idx: BufferIndex,
    {
        self.0.search(iter, regex)
    }
}

pub enum InflightSearch {
    /// The indexing process is incomplete. The process must be started using
    /// the associated [InflightIndexRemote]. Accesses to the data inside
    /// are relatively cheap, with atomic copies of the ref-counted pointers
    /// to the internal buffers.
    Incomplete(#[doc(hidden)] Arc<InflightSearchImpl>),
    /// The indexing process is finalized, and the internal representation is
    /// replaced with a [CompleteIndex]. This can be obtained through
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
        (
            Self::Incomplete(inner.clone()),
            InflightSearchRemote(inner),
        )
    }

    /// Searches for a regular expression pattern in a sharded buffer.
    ///
    /// # Arguments
    ///
    /// * `buf` - The sharded buffer to search in.
    /// * `regex` - The regular expression pattern to search for.
    ///
    /// # Returns
    ///
    /// Returns a `Result` containing the `InflightSearch` object if the search is successful,
    /// or an error if the search fails.
    ///
    /// # Generic Parameters
    ///
    /// * `Idx` - The type of the buffer index.
    pub fn search<Idx>(buf: &ShardedBuffer<Idx>, regex: Regex) -> Result<Self>
    where
        Idx: BufferIndex + Clone + Send + 'static,
    {
        let (search, remote) = InflightSearch::new();
        std::thread::spawn({
            let iter = buf.multibuffer_iter()?;
            move || remote.search(iter, regex)
        });
        Ok(search)
    }
}

impl BufferSearch for InflightSearch {
    fn find(&self, line_number: usize) -> bool {
        match self {
            InflightSearch::Incomplete(inner) => inner.read(|index| index.find(line_number)),
            InflightSearch::Complete(index) => index.find(line_number),
        }
    }
}
