use std::fs::File as StdFile;
use std::ops::Range;
use std::os::fd::AsRawFd;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use anyhow::Result;
use tokio::fs::File as TokioFile;
use tokio::sync::mpsc::Receiver;

use super::partition::RangePartition;

struct IndexingTask {
    sx: tokio::sync::mpsc::Sender<u64>,
    data: memmap2::Mmap,
    start: u64,
}

impl IndexingTask {
    const LINES_PER_MB: usize = 1 << 13;

    fn get_data<T: AsRawFd>(file: &T, start: u64, end: u64) -> Result<memmap2::Mmap> {
        let data = unsafe {
            memmap2::MmapOptions::new()
                .offset(start)
                .len((end - start) as usize)
                .map(file)?
        };
        data.advise(memmap2::Advice::Sequential)?;
        Ok(data)
    }

    fn new<T: AsRawFd>(file: &T, start: u64, end: u64) -> Result<(Self, Receiver<u64>)> {
        let data = Self::get_data(file, start, end)?;
        let (sx, rx) = tokio::sync::mpsc::channel(Self::LINES_PER_MB);
        Ok((Self { sx, data, start }, rx))
    }

    async fn worker_async(self) -> Result<()> {
        for i in memchr::memchr_iter(b'\n', &self.data) {
            self.sx.send(self.start + i as u64).await?;
        }

        Ok(())
    }
}

#[derive(Debug)]
pub struct IncompleteIndex {
    inner: FileIndex,
    data_size: u64,
    finished: bool,
}

impl IncompleteIndex {
    /// Constant to determine how much data is stored in each shard.
    /// Note that the shard likely contains more than the threshold,
    /// this is just a cutoff.
    const SHARD_DATA_THRESHOLD: u64 = 1 << 20;

    /// How much data of the file should each indexing task handle?
    const INDEXING_VIEW_SIZE: u64 = 1 << 20;

    pub fn new() -> Self {
        Self {
            inner: FileIndex::empty(),
            data_size: 0,
            finished: false,
        }
    }

    pub fn index(mut self, file: &StdFile) -> Result<FileIndex> {
        let len = file.metadata()?.len();
        let mut start = 0;

        while start < len {
            let end = (start + IncompleteIndex::INDEXING_VIEW_SIZE).min(len);

            let data = unsafe {
                memmap2::MmapOptions::new()
                    .offset(start)
                    .len((end - start) as usize)
                    .map(file)?
            };
            data.advise(memmap2::Advice::Sequential)?;

            for i in memchr::memchr_iter(b'\n', &data) {
                let line_data = start + i as u64;
                self.push_line_data(line_data);
            }

            start = end;
        }
        self.finalize(len);

        Ok(self.inner)
    }

    fn push_line_data(&mut self, line_data: u64) {
        let line_number = self.inner.line_index.len();
        self.inner.line_index.push(line_data);

        self.data_size += line_data;
        if self.data_size > Self::SHARD_DATA_THRESHOLD {
            self.inner.shard_partition.partition(line_number);
            self.data_size = 0;
        }
    }

    fn finalize(&mut self, len: u64) {
        self.inner.line_index.push(len);
        // In case the shard boundary did not end on the very last line we iterated through
        self.inner
            .shard_partition
            .partition(self.inner.line_index.len() - 1);

        self.finished = true;
    }

    fn finish(self) -> FileIndex {
        assert!(self.finished);
        self.inner
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct FileIndex {
    /// Store the byte location of the start of the indexed line
    line_index: Vec<u64>,
    /// Allow queries from line number in a line range to shard
    shard_partition: RangePartition,
}

impl FileIndex {
    fn empty() -> Self {
        Self {
            line_index: vec![0],
            shard_partition: RangePartition::new(),
        }
    }

    pub fn line_count(&self) -> usize {
        self.line_index.len() - 2
    }

    pub fn shard_count(&self) -> usize {
        self.shard_partition.len()
    }

    pub fn start_of_line(&self, line_number: usize) -> u64 {
        self.line_index[line_number]
    }

    pub fn shard_of_line(&self, line_number: usize) -> Option<usize> {
        self.shard_partition.lookup(line_number)
    }

    pub fn translate_data_from_line_range(&self, line_range: Range<usize>) -> Range<u64> {
        self.start_of_line(line_range.start)..self.start_of_line(line_range.end)
    }

    pub fn line_range_of_shard(&self, shard_id: usize) -> Option<Range<usize>> {
        self.shard_partition.reverse_lookup(shard_id)
    }

    pub fn data_range_of_shard(&self, shard_id: usize) -> Option<Range<u64>> {
        Some(self.translate_data_from_line_range(self.line_range_of_shard(shard_id)?))
    }
}

#[derive(Debug)]
pub struct AsyncIndexImpl {
    inner: tokio::sync::Mutex<IncompleteIndex>,
    progress: AtomicU64,
}

impl AsyncIndexImpl {
    fn new() -> Arc<Self> {
        Arc::new(AsyncIndexImpl {
            inner: tokio::sync::Mutex::new(IncompleteIndex::new()),
            progress: AtomicU64::new(0),
        })
    }

    async fn index(self: Arc<Self>, file: TokioFile) -> Result<()> {
        assert_eq!(Arc::strong_count(&self), 2);
        // Build line & shard index
        let (sx, mut rx) = tokio::sync::mpsc::channel(4);

        let len = file.metadata().await?.len();
        let file = file.try_clone().await?;

        // Indexing worker
        let spawner = tokio::task::spawn(async move {
            let mut curr = 0;

            while curr < len {
                let end = (curr + IncompleteIndex::INDEXING_VIEW_SIZE).min(len);
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

    pub fn progress(&self) -> f64 {
        f64::from_bits(self.progress.load(std::sync::atomic::Ordering::SeqCst))
    }
}

pub struct AsyncIndexIndexer(Arc<AsyncIndexImpl>);

impl AsyncIndexIndexer {
    pub async fn index(self, file: TokioFile) -> Result<()> {
        self.0.index(file).await
    }
}

#[derive(Debug)]
pub enum AsyncIndex {
    Incomplete(Arc<AsyncIndexImpl>),
    Complete(FileIndex),
}

impl AsyncIndex {
    pub fn new() -> (Self, AsyncIndexIndexer) {
        let inner = AsyncIndexImpl::new();
        (Self::Incomplete(inner.clone()), AsyncIndexIndexer(inner))
    }

    pub async fn new_complete(file: &TokioFile) -> Result<FileIndex> {
        let (result, indexer) = Self::new();
        indexer.index(file.try_clone().await?).await?;
        Ok(result.unwrap())
    }

    pub async fn read<F, T>(&self, cb: F) -> T
    where
        F: FnOnce(&FileIndex) -> T,
    {
        match self {
            AsyncIndex::Incomplete(indexer) => cb(&indexer.inner.lock().await.inner),
            AsyncIndex::Complete(index) => cb(index),
        }
    }

    /// Use this to drop the indirection
    pub fn try_finalize(&mut self) -> bool {
        match self {
            AsyncIndex::Incomplete(inner) if Arc::strong_count(inner) == 1 => {
                let inner = unsafe {
                    Arc::try_unwrap(std::mem::replace(inner, AsyncIndexImpl::new()))
                        .unwrap_unchecked()
                };
                *self = AsyncIndex::Complete(inner.inner.into_inner().finish());
                true
            }
            AsyncIndex::Incomplete(_) => false,
            AsyncIndex::Complete(_) => true,
        }
    }

    fn unwrap(mut self) -> FileIndex {
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

    pub fn progress(&self) -> f64 {
        match self {
            AsyncIndex::Incomplete(indexer) => indexer.progress(),
            AsyncIndex::Complete(_) => 1.0,
        }
    }

    pub async fn line_count(&self) -> usize {
        self.read(|index| index.line_count()).await
    }

    pub async fn shard_count(&self) -> usize {
        self.read(|index| index.shard_count()).await
    }

    pub async fn start_of_line(&self, line_number: usize) -> u64 {
        self.read(|index| index.start_of_line(line_number)).await
    }

    pub async fn shard_of_line(&self, line_number: usize) -> Option<usize> {
        self.read(|index| index.shard_of_line(line_number)).await
    }

    pub async fn translate_data_from_line_range(&self, line_range: Range<usize>) -> Range<u64> {
        self.read(|index| index.translate_data_from_line_range(line_range))
            .await
    }

    pub async fn line_range_of_shard(&self, shard_id: usize) -> Option<Range<usize>> {
        self.read(|index| index.line_range_of_shard(shard_id)).await
    }

    pub async fn data_range_of_shard(&self, shard_id: usize) -> Option<Range<u64>> {
        self.read(|index| index.data_range_of_shard(shard_id)).await
    }
}
