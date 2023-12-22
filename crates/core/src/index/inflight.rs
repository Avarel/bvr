//! Contains the [InflightIndex] and [InflightIndexRemote], which are abstractions
//! that allow the use of [IncompleteIndex] functionalities while it is "inflight"
//! or in the middle of the indexing operation.

use super::{BufferIndex, CompleteIndex, IncompleteIndex};
use crate::{
    buf::segment::{Segment, SegmentMut},
    err::{Error, Result},
};
use std::sync::mpsc::{Receiver, Sender};
use std::thread::JoinHandle;
use std::{
    fs::File,
    sync::{atomic::AtomicU64, Arc},
};

/// Internal indexing task used by [InflightIndexImpl].
struct IndexingTask {
    /// This is the sender side of the channel that receives byte indexes of `\n`.
    sx: Sender<u64>,
    segment: Segment,
}

impl IndexingTask {
    fn new(file: &File, start: u64, end: u64) -> Result<(Self, Receiver<u64>)> {
        let segment = Segment::map_file(start..end, file)?;
        let (sx, rx) = std::sync::mpsc::channel();
        Ok((Self { sx, segment }, rx))
    }

    fn compute(self) -> Result<()> {
        for i in memchr::memchr_iter(b'\n', &self.segment) {
            self.sx
                .send(self.segment.start() + i as u64 + 1)
                .map_err(|_| Error::Internal)?;
        }

        Ok(())
    }
}

#[doc(hidden)]
pub struct InflightIndexImpl {
    inner: std::sync::Mutex<IncompleteIndex>,
    cache: std::sync::Mutex<Option<CompleteIndex>>,
    progress: AtomicU64,
    mode: InflightIndexMode,
}

/// Progress report by [InflightIndex]'s `progress()` method.
pub enum InflightIndexProgress {
    /// The indexing process is complete. This value can only be returned if
    /// `InflightIndex::try_finalize` has returned true.
    Done,
    /// The indexing process is working with a stream. There is no known end
    /// to the stream, just that it is working through the stream.
    Streaming,
    /// The indexing process is working with a file. There is a known end
    /// to the file, and the float value is bounded between `0.0..1.0` and
    /// represents the progress made on the file.
    File(f64),
}

/// The mode to be used by the [InflightIndexRemote]. This has no effect
/// besides contraining what the [InflightIndexRemote] can be used for
/// and progress reports from [`InflightIndex::progress()`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InflightIndexMode {
    /// The [InflightIndexRemote] can only be used to index files.
    /// The progress reports are bounded between `0.0..1.0` and
    /// represents the progress made on the file.
    Stream,
    /// The [InflightIndexRemote] can only be used to index streams.
    /// There are no progress reports for this mode.
    File,
}

/// Generalized type for streams passed into [InflightIndexRemote].
pub type Stream = Box<dyn std::io::Read + Send>;

impl InflightIndexImpl {
    fn new(mode: InflightIndexMode) -> Arc<Self> {
        Arc::new(InflightIndexImpl {
            inner: std::sync::Mutex::new(IncompleteIndex::new()),
            progress: AtomicU64::new(0),
            cache: std::sync::Mutex::new(None),
            mode,
        })
    }

    fn index_file(self: Arc<Self>, file: File) -> Result<()> {
        assert_eq!(self.mode, InflightIndexMode::File);
        assert!(Arc::strong_count(&self) >= 2);
        // Build index
        let (sx, rx) = std::sync::mpsc::sync_channel(4);

        let len = file.metadata()?.len();
        let file = file.try_clone()?;

        // Indexing worker
        let spawner: JoinHandle<Result<()>> = std::thread::spawn(move || {
            let mut curr = 0;

            while curr < len {
                let end = (curr + Segment::MAX_SIZE).min(len);
                let (task, task_rx) = IndexingTask::new(&file, curr, end)?;
                sx.send(task_rx).unwrap();

                std::thread::spawn(|| task.compute());

                curr = end;
            }

            Ok(())
        });

        while let Ok(task_rx) = rx.recv() {
            while let Ok(line_data) = task_rx.recv() {
                let mut inner = self.inner.lock().unwrap();
                inner.push_line_data(line_data);
                self.progress.store(
                    (line_data as f64 / len as f64).to_bits(),
                    std::sync::atomic::Ordering::SeqCst,
                );
            }
        }

        spawner.join().unwrap().unwrap();
        let mut inner = self.inner.lock().unwrap();
        inner.finalize(len);
        Ok(())
    }

    fn index_stream(self: Arc<Self>, mut stream: Stream, outgoing: Sender<Segment>) -> Result<()> {
        assert_eq!(self.mode, InflightIndexMode::Stream);
        let mut len = 0;

        loop {
            let mut segment = SegmentMut::new(len)?;

            let mut buf_len = 0;
            loop {
                match stream.read(&mut segment[buf_len..])? {
                    0 => break,
                    l => buf_len += l,
                }
            }

            let mut inner = self.inner.lock().unwrap();
            for i in memchr::memchr_iter(b'\n', &segment) {
                let line_data = len + i as u64;
                inner.push_line_data(line_data + 1);
            }

            outgoing
                .send(segment.into_read_only()?)
                .map_err(|_| Error::Internal)?;

            if buf_len == 0 {
                break;
            }

            len += buf_len as u64;
        }

        let mut inner = self.inner.lock().unwrap();
        inner.finalize(len);
        Ok(())
    }

    fn progress(&self) -> InflightIndexProgress {
        match self.mode {
            InflightIndexMode::Stream => InflightIndexProgress::Streaming,
            InflightIndexMode::File => InflightIndexProgress::File(f64::from_bits(
                self.progress.load(std::sync::atomic::Ordering::SeqCst),
            )),
        }
    }

    fn read<F, T>(&self, cb: F) -> T
    where
        F: FnOnce(&CompleteIndex) -> T,
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

/// A remote type that can be used to set off the indexing process of a
/// file or a stream.
pub struct InflightIndexRemote(Arc<InflightIndexImpl>);

impl InflightIndexRemote {
    /// Index a file and load the data into the associated [InflightIndex].
    pub fn index_file(self, file: File) -> Result<()> {
        self.0.index_file(file)
    }

    /// Index a stream and load the data into the associated [InflightIndex].
    pub fn index_stream(self, stream: Stream, outgoing: Sender<Segment>) -> Result<()> {
        self.0.index_stream(stream, outgoing)
    }
}

/// An index that may be "inflight." This means that the information in this
/// index may be incomplete and in the middle of processing.
///
/// However, the present data is still reliable, just that it may not represent
/// the complete picture.
pub enum InflightIndex {
    /// The indexing process is incomplete. The process must be started using
    /// the associated [InflightIndexRemote]. Accesses to the data inside
    /// are relatively cheap, with atomic copies of the ref-counted pointers
    /// to the internal buffers.
    Incomplete(#[doc(hidden)] Arc<InflightIndexImpl>),
    /// The indexing process is finalized, and the internal representation is
    /// replaced with a [CompleteIndex]. This can be obtained through
    /// [`Self::try_finalize()`].
    Complete(CompleteIndex),
}

impl InflightIndex {
    /// This function creates a new instance of [InflightIndex] and its associated [InflightIndexRemote].
    /// The [InflightIndex] is responsible for managing the state of an ongoing indexing operation,
    /// while the [InflightIndexRemote] provides a remote interface for starting off the indexing operation.
    ///
    /// # Examples
    ///
    /// ```
    /// use bvr_core::index::inflight::{InflightIndex, InflightIndexMode};
    ///
    /// let inflight_index = InflightIndex::new(InflightIndexMode::File);
    /// ```
    ///
    /// # Returns
    ///
    /// A tuple containing the newly created `InflightIndex` and the associated `InflightIndexRemote`.
    pub fn new(mode: InflightIndexMode) -> (Self, InflightIndexRemote) {
        let inner = InflightIndexImpl::new(mode);
        (Self::Incomplete(inner.clone()), InflightIndexRemote(inner))
    }

    /// Creates a new complete index from a file.
    ///
    /// This function creates a new complete index from the provided file. It internally
    /// initializes an [InflightIndex] in file mode, indexes the file, and returns the
    /// resulting [CompleteIndex].
    ///
    /// # Returns
    ///
    /// A `Result` containing the `CompleteIndex` if successful, or an error if the
    /// indexing process fails.
    pub fn new_complete(file: &File) -> Result<CompleteIndex> {
        let (result, indexer) = Self::new(InflightIndexMode::File);
        indexer.index_file(file.try_clone()?)?;
        Ok(result.unwrap())
    }

    /// Transparently replace inner atomically ref-counted [IncompleteIndex]
    /// with a [CompleteIndex]. If the function is successful, future accesses
    /// will not pay the cost of atomics and mutexes to access the inner data of this index.
    ///
    /// This function cannot succeed until the associated [InflightIndexRemote]
    /// has been dropped.
    pub fn try_finalize(&mut self) -> bool {
        match self {
            Self::Incomplete(inner) => {
                match Arc::try_unwrap(std::mem::replace(
                    inner,
                    InflightIndexImpl::new(InflightIndexMode::File),
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

    /// Unwrap the [InflightIndex] into a [CompleteIndex]. This method panics if
    /// [`Self::try_finalize()`] fails.
    pub fn unwrap(mut self) -> CompleteIndex {
        match self {
            Self::Incomplete { .. } => {
                if self.try_finalize() {
                    self.unwrap()
                } else {
                    panic!("indexing is incomplete")
                }
            }
            Self::Complete(inner) => inner,
        }
    }

    /// Returns the inflight index's progress.
    pub fn progress(&self) -> InflightIndexProgress {
        match self {
            InflightIndex::Incomplete(indexer) => indexer.progress(),
            InflightIndex::Complete(_) => InflightIndexProgress::Done,
        }
    }
}

impl Clone for InflightIndex {
    fn clone(&self) -> Self {
        match self {
            Self::Incomplete(inner) => inner.read(|index| Self::Complete(index.clone())),
            Self::Complete(index) => Self::Complete(index.clone()),
        }
    }
}

macro_rules! demux {
    ($exp:expr, $index: pat, $s:expr) => {
        match $exp {
            Self::Incomplete(inner) => inner.read(|$index| $s),
            Self::Complete($index) => $s,
        }
    };
}
impl BufferIndex for InflightIndex {
    fn line_count(&self) -> usize {
        demux!(self, index, index.line_count())
    }

    fn data_of_line(&self, line_number: usize) -> Option<u64> {
        demux!(self, index, index.data_of_line(line_number))
    }

    fn line_of_data(&self, start: u64) -> Option<usize> {
        demux!(self, index, index.line_of_data(start))
    }
}
