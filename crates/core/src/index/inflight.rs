use crate::buf::shard::Shard;

use super::{CompleteIndex, BufferIndex, IncompleteIndex};

use anyhow::Result;
use tokio::sync::mpsc::{Receiver, Sender};
use std::{sync::{atomic::AtomicU64, Arc}, fs::File};

struct IndexingTask {
    /// This is the sender side of the channel that receives byte indexes of `\n`.
    sx: Sender<u64>,
    /// Memmap buffer.
    data: memmap2::Mmap,
    /// Indicates where the buffer starts within the file.
    start: u64,
}

impl IndexingTask {
    const HEURISTIC_LINES_PER_MB: usize = 1 << 13;

    fn map<T: crate::Mmappable>(file: &T, start: u64, end: u64) -> Result<memmap2::Mmap> {
        let data = unsafe {
            memmap2::MmapOptions::new()
                .offset(start)
                .len((end - start) as usize)
                .map(file)?
        };
        #[cfg(unix)]
        data.advise(memmap2::Advice::Sequential)?;
        Ok(data)
    }

    fn new<T: crate::Mmappable>(file: &T, start: u64, end: u64) -> Result<(Self, Receiver<u64>)> {
        let data = Self::map(file, start, end)?;
        let (sx, rx) = tokio::sync::mpsc::channel(Self::HEURISTIC_LINES_PER_MB);
        Ok((Self { sx, data, start }, rx))
    }

    async fn worker_async(self) -> Result<()> {
        for i in memchr::memchr_iter(b'\n', &self.data) {
            self.sx.send(self.start + i as u64 + 1).await?;
        }

        Ok(())
    }
}

pub struct InflightIndexImpl {
    inner: tokio::sync::Mutex<IncompleteIndex>,
    progress: AtomicU64,
    cache: std::sync::Mutex<Option<CompleteIndex>>,
    mode: InflightIndexMode,
}

/// Inflight index's progress.
pub enum InflightIndexProgress {
    Done,
    Streaming,
    File(f64),
}

/// Mainly used for progress reports.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InflightIndexMode {
    Stream,
    File,
}

pub type InflightStream = Box<dyn std::io::Read + Send>;

impl InflightIndexImpl {
    fn new(mode: InflightIndexMode) -> Arc<Self> {
        Arc::new(InflightIndexImpl {
            inner: tokio::sync::Mutex::new(IncompleteIndex::new()),
            progress: AtomicU64::new(0),
            cache: std::sync::Mutex::new(None),
            mode,
        })
    }

    async fn index_file(self: Arc<Self>, file: File) -> Result<()> {
        assert_eq!(self.mode, InflightIndexMode::File);
        assert_eq!(Arc::strong_count(&self), 2);
        // Build line & shard index
        let (sx, mut rx) = tokio::sync::mpsc::channel(4);

        let len = file.metadata()?.len();
        let file = file.try_clone()?;

        // Indexing worker
        let spawner = tokio::task::spawn(async move {
            let mut curr = 0;

            while curr < len {
                let end = (curr + crate::INDEXING_VIEW_SIZE).min(len);
                let (task, task_rx) = IndexingTask::new(&file, curr, end)?;
                sx.send(task_rx).await?;
                tokio::task::spawn(task.worker_async());

                curr = end;
            }

            Ok::<(), anyhow::Error>(())
        });

        while let Some(mut task_rx) = rx.recv().await {
            while let Some(line_data) = task_rx.recv().await {
                let mut inner = self.inner.lock().await;
                inner.push_line_data(line_data);
                // Poll for more data to avoid locking and relocking
                for _ in 0..IndexingTask::HEURISTIC_LINES_PER_MB {
                    if let Ok(line_data) = task_rx.try_recv() {
                        inner.push_line_data(line_data);
                    } else {
                        break;
                    }
                }
                self.progress.store(
                    (line_data as f64 / len as f64).to_bits(),
                    std::sync::atomic::Ordering::SeqCst,
                );
            }
        }

        spawner.await??;
        let mut inner = self.inner.lock().await;
        Ok(inner.finalize(len))
    }

    async fn index_stream(
        self: Arc<Self>,
        mut stream: InflightStream,
        outgoing: Sender<Shard>,
    ) -> Result<()> {
        assert_eq!(self.mode, InflightIndexMode::Stream);
        let mut len = 0;
        let mut shard_id = 0;

        loop {
            let mut data = memmap2::MmapOptions::new()
                .len(crate::INDEXING_VIEW_SIZE as usize)
                .map_anon()?;
            #[cfg(unix)]
            data.advise(memmap2::Advice::Sequential)?;

            let mut buf_start = 0;
            loop {
                match stream.read(&mut data[buf_start..crate::INDEXING_VIEW_SIZE as usize])? {
                    0 => break,
                    l => buf_start += l,
                }
            };
            if buf_start == 0 {
                break;
            }

            let mut inner = self.inner.lock().await;
            for i in memchr::memchr_iter(b'\n', &data) {
                let line_data = len + i as u64;
                inner.push_line_data(line_data + 1);
            }

            outgoing
                .send(Shard::new(shard_id, len, data.make_read_only()?))
                .await?;

            shard_id += 1;
            len += buf_start as u64;
        }

        let mut inner = self.inner.lock().await;
        Ok(inner.finalize(len))
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
        T: std::fmt::Debug + Clone,
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

                let clone = self.inner.blocking_lock().inner.clone();
                let val = cb(&clone);
                *self.cache.lock().unwrap() = Some(clone);
                val
            }
        }
    }
}

impl BufferIndex for InflightIndexImpl {
    fn line_count(&self) -> usize {
        self.read(|index| index.line_count())
    }

    fn data_of_line(&self, line_number: usize) -> Option<u64> {
        self.read(|index| index.data_of_line(line_number))
    }

    fn line_of_data(&self, start: u64) -> Option<usize> {
        self.read(|index| index.line_of_data(start))
    }
}

pub struct InflightIndexIndexer(Arc<InflightIndexImpl>);

impl InflightIndexIndexer {
    pub(crate) async fn index_file(self, file: File) -> Result<()> {
        self.0.index_file(file).await
    }

    pub(crate) async fn index_stream(
        self,
        stream: InflightStream,
        outgoing: Sender<Shard>,
    ) -> Result<()> {
        self.0.index_stream(stream, outgoing).await
    }
}

pub enum InflightIndex {
    Incomplete(Arc<InflightIndexImpl>),
    Complete(CompleteIndex),
}

impl InflightIndex {
    /// Create an empty inflight index with a remote that can be used
    /// to set off the indexing process asyncronously.
    pub fn new(mode: InflightIndexMode) -> (Self, InflightIndexIndexer) {
        let inner = InflightIndexImpl::new(mode);
        (Self::Incomplete(inner.clone()), InflightIndexIndexer(inner))
    }

    /// Create an index and drive it to completion using inflight async mechanisms.
    pub async fn new_complete(file: &File) -> Result<CompleteIndex> {
        let (result, indexer) = Self::new(InflightIndexMode::File);
        indexer.index_file(file.try_clone()?).await?;
        Ok(result.unwrap())
    }

    /// Transparently replace inner atomically ref-counted [IncompleteIndex] with a [CompleteIndex].
    /// If the function is successful, future accesses will not pay the cost of atomics
    /// and mutexes to access the inner data of this index.
    pub fn try_finalize(&mut self) -> bool {
        match self {
            InflightIndex::Incomplete(inner) if Arc::strong_count(inner) == 1 => {
                let inner = unsafe {
                    Arc::try_unwrap(std::mem::replace(
                        inner,
                        InflightIndexImpl::new(InflightIndexMode::File),
                    ))
                    .unwrap_unchecked()
                };
                *self = InflightIndex::Complete(inner.inner.into_inner().finish());
                true
            }
            InflightIndex::Incomplete(_) => false,
            InflightIndex::Complete(_) => true,
        }
    }

    pub(crate) fn unwrap(mut self) -> CompleteIndex {
        match self {
            InflightIndex::Incomplete { .. } => {
                if self.try_finalize() {
                    self.unwrap()
                } else {
                    panic!("indexing is incomplete")
                }
            }
            InflightIndex::Complete(inner) => inner,
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

macro_rules! demux {
    ($exp:expr, $index: pat, $s:expr) => {
        match $exp {
            Self::Incomplete($index) => $s,
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
