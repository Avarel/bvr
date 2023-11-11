use crate::file::shard::Shard;

use super::{CompleteIndex, FileIndex, IncompleteIndex, IndexingTask};

use anyhow::Result;
use std::ops::Range;
use std::sync::{atomic::AtomicU64, Arc};

pub struct AsyncIndexImpl {
    inner: tokio::sync::Mutex<IncompleteIndex>,
    progress: AtomicU64,
    cache: std::sync::Mutex<Option<CompleteIndex>>,
    mode: AsyncIndexMode,
}

pub enum AsyncIndexProgress {
    Done,
    Streaming,
    File(f64),
}

#[derive(Clone, Copy)]
pub enum AsyncIndexMode {
    Stream,
    File,
}

pub type AsyncStream = Box<dyn std::io::Read + Send>;

impl AsyncIndexImpl {
    pub(crate) fn new(mode: AsyncIndexMode) -> Arc<Self> {
        Arc::new(AsyncIndexImpl {
            inner: tokio::sync::Mutex::new(IncompleteIndex::new()),
            progress: AtomicU64::new(0),
            cache: std::sync::Mutex::new(None),
            mode,
        })
    }

    async fn index_file(self: Arc<Self>, file: tokio::fs::File) -> Result<()> {
        assert_eq!(Arc::strong_count(&self), 2);
        // Build line & shard index
        let (sx, mut rx) = tokio::sync::mpsc::channel(4);

        let len = file.metadata().await?.len();
        let file = file.try_clone().await?;

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
                for _ in 0..IndexingTask::LINES_PER_MB {
                    if let Ok(line_data) = task_rx.try_recv() {
                        inner.push_line_data(line_data + 1);
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
        mut stream: AsyncStream,
        outgoing: tokio::sync::mpsc::Sender<Shard>,
    ) -> Result<()> {
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

    pub fn progress(&self) -> AsyncIndexProgress {
        match self.mode {
            AsyncIndexMode::Stream => AsyncIndexProgress::Streaming,
            AsyncIndexMode::File => AsyncIndexProgress::File(f64::from_bits(
                self.progress.load(std::sync::atomic::Ordering::SeqCst),
            )),
        }
    }

    pub(crate) fn read<F, T>(&self, cb: F) -> T
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

impl FileIndex for AsyncIndexImpl {
    fn line_count(&self) -> usize {
        self.read(|index| index.line_count())
    }

    fn shard_count(&self) -> usize {
        self.read(|index| index.shard_count())
    }

    fn start_of_line(&self, line_number: usize) -> u64 {
        self.read(|index| index.start_of_line(line_number))
    }

    fn shard_of_line(&self, line_number: usize) -> Option<usize> {
        self.read(|index| index.shard_of_line(line_number))
    }

    fn translate_data_from_line_range(&self, line_range: Range<usize>) -> Range<u64> {
        self.read(|index| index.translate_data_from_line_range(line_range))
    }

    fn line_range_of_shard(&self, shard_id: usize) -> Option<Range<usize>> {
        self.read(|index| index.line_range_of_shard(shard_id))
    }

    fn data_range_of_shard(&self, shard_id: usize) -> Option<Range<u64>> {
        self.read(|index| index.data_range_of_shard(shard_id))
    }
}

pub struct AsyncIndexIndexer(Arc<AsyncIndexImpl>);

impl AsyncIndexIndexer {
    pub async fn index_file(self, file: tokio::fs::File) -> Result<()> {
        self.0.index_file(file).await
    }

    pub async fn index_stream(
        self,
        stream: AsyncStream,
        outgoing: tokio::sync::mpsc::Sender<Shard>,
    ) -> Result<()> {
        self.0.index_stream(stream, outgoing).await
    }
}

pub enum AsyncIndex {
    Incomplete(Arc<AsyncIndexImpl>),
    Complete(CompleteIndex),
}

impl AsyncIndex {
    pub fn new(mode: AsyncIndexMode) -> (Self, AsyncIndexIndexer) {
        let inner = AsyncIndexImpl::new(mode);
        (Self::Incomplete(inner.clone()), AsyncIndexIndexer(inner))
    }

    pub async fn new_complete(file: &tokio::fs::File) -> Result<CompleteIndex> {
        let (result, indexer) = Self::new(AsyncIndexMode::File);
        indexer.index_file(file.try_clone().await?).await?;
        Ok(result.unwrap())
    }

    /// Transparently replace inner implementation with
    pub fn try_finalize(&mut self) -> bool {
        match self {
            AsyncIndex::Incomplete(inner) if Arc::strong_count(inner) == 1 => {
                let inner = unsafe {
                    Arc::try_unwrap(std::mem::replace(
                        inner,
                        AsyncIndexImpl::new(AsyncIndexMode::File),
                    ))
                    .unwrap_unchecked()
                };
                *self = AsyncIndex::Complete(inner.inner.into_inner().finish());
                true
            }
            AsyncIndex::Incomplete(_) => false,
            AsyncIndex::Complete(_) => true,
        }
    }

    pub(crate) fn unwrap(mut self) -> CompleteIndex {
        match self {
            AsyncIndex::Incomplete { .. } => {
                if self.try_finalize() {
                    self.unwrap()
                } else {
                    panic!("indexing is incomplete")
                }
            }
            AsyncIndex::Complete(inner) => inner,
        }
    }

    pub fn progress(&self) -> AsyncIndexProgress {
        match self {
            AsyncIndex::Incomplete(indexer) => indexer.progress(),
            AsyncIndex::Complete(_) => AsyncIndexProgress::Done,
        }
    }
}

macro_rules! demux {
    ($exp:expr, $index: pat, $s:expr) => {
        match $exp {
            AsyncIndex::Incomplete($index) => $s,
            AsyncIndex::Complete($index) => $s,
        }
    };
}

impl FileIndex for AsyncIndex {
    fn line_count(&self) -> usize {
        demux!(self, index, index.line_count())
    }

    fn shard_count(&self) -> usize {
        demux!(self, index, index.shard_count())
    }

    fn start_of_line(&self, line_number: usize) -> u64 {
        demux!(self, index, index.start_of_line(line_number))
    }

    fn shard_of_line(&self, line_number: usize) -> Option<usize> {
        demux!(self, index, index.shard_of_line(line_number))
    }

    fn translate_data_from_line_range(&self, line_range: Range<usize>) -> Range<u64> {
        demux!(
            self,
            index,
            index.translate_data_from_line_range(line_range)
        )
    }

    fn line_range_of_shard(&self, shard_id: usize) -> Option<Range<usize>> {
        demux!(self, index, index.line_range_of_shard(shard_id))
    }
}
