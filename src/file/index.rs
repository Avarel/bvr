use std::ops::Range;
use std::os::fd::AsRawFd;

use anyhow::Result;
use tokio::fs::File as TokioFile;
use tokio::sync::mpsc::Receiver;

#[cfg(test)]
use std::fs::File as StdFile;

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

pub(super) struct IncompleteIndex {
    index: FileIndex,
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

    fn new() -> Self {
        Self {
            index: FileIndex::empty(),
            data_size: 0,
            finished: false,
        }
    }

    /// Use to check correctness of async version
    #[cfg(test)]
    fn index_sync(&mut self, file: &StdFile) -> Result<()> {
        let len = file.metadata()?.len();
        let mut start = 0;

        while start < len {
            let end = (start + IncompleteIndex::INDEXING_VIEW_SIZE).min(len);

            let data = IndexingTask::get_data(file, start, end)?;
            for i in memchr::memchr_iter(b'\n', &data) {
                let line_data = start + i as u64;
                self.push_line_data(line_data);
            }

            start = end;
        }
        Ok(self.finalize(len))
    }

    fn push_line_data(&mut self, line_data: u64) {
        let line_number = self.index.line_index.len();
        self.index.line_index.push(line_data);

        self.data_size += line_data;
        if self.data_size > Self::SHARD_DATA_THRESHOLD {
            self.index.shard_partition.partition(line_number);
            self.data_size = 0;
        }
    }

    fn finalize(&mut self, len: u64) {
        self.index.line_index.push(len);
        // In case the shard boundary did not end on the very last line we iterated through
        self.index
            .shard_partition
            .partition(self.index.line_index.len() - 1);

        self.finished = true;
    }

    fn finish(self) -> FileIndex {
        assert!(self.finished);
        self.index
    }
}

pub(super) enum AsyncIndex {
    Incomplete(tokio::sync::Mutex<IncompleteIndex>),
    Complete(FileIndex),
}

impl AsyncIndex {
    fn new() -> Self {
        Self::Incomplete(tokio::sync::Mutex::new(IncompleteIndex::new()))
    }

    async fn index(&self, file: &TokioFile) -> Result<()> {
        // Build line & shard index
        let (sx, mut rx) = tokio::sync::mpsc::channel(10);

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
                self.write(|z| {
                    z.push_line_data(line_data);
                    // Poll for more data to avoid locking and relocking
                    for _ in 0..IndexingTask::LINES_PER_MB {
                        if let Ok(line_data) = task_rx.try_recv() {
                            z.push_line_data(line_data);
                        } else {
                            break;
                        }
                    }
                })
                .await;
            }
        }
        spawner.await??;
        Ok(self.write(|z| z.finalize(len)).await)
    }

    async fn write<F>(&self, cb: F)
    where
        F: FnOnce(&mut IncompleteIndex),
    {
        match self {
            AsyncIndex::Incomplete(inner) => cb(&mut *inner.lock().await),
            AsyncIndex::Complete(_) => panic!("Index is already complete!"),
        }
    }

    async fn read<F, T>(&self, cb: F) -> T
    where
        F: FnOnce(&FileIndex) -> T,
    {
        match self {
            AsyncIndex::Incomplete(inner) => cb(&inner.lock().await.index),
            AsyncIndex::Complete(index) => cb(index),
        }
    }

    fn try_finalize(&mut self) -> bool {
        match self {
            AsyncIndex::Incomplete(inner) => {
                let inner = inner.get_mut();
                if !inner.finished {
                    return false;
                }
                *self =
                    AsyncIndex::Complete(std::mem::replace(inner, IncompleteIndex::new()).finish())
            }
            AsyncIndex::Complete(_) => {}
        }
        true
    }

    fn into_inner(self) -> FileIndex {
        match self {
            AsyncIndex::Incomplete(_) => panic!("The index is incomplete!"),
            AsyncIndex::Complete(inner) => inner,
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

#[derive(Debug, PartialEq, Eq)]
pub(super) struct FileIndex {
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

    pub(super) async fn new(file: &TokioFile) -> Result<Self> {
        let mut result = AsyncIndex::new();
        result.index(file).await?;
        assert!(result.try_finalize());
        Ok(result.into_inner())
    }

    #[cfg(test)]
    pub(super) fn new_sync(file: &StdFile) -> Result<Self> {
        let mut result = IncompleteIndex::new();
        result.index_sync(file)?;
        Ok(result.finish())
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
        self.line_range_of_shard(shard_id)
            .map(|range| self.translate_data_from_line_range(range))
    }
}

#[cfg(test)]
mod test {
    use super::FileIndex;

    #[test]
    fn what() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let file = rt.block_on(tokio::fs::File::open("./Cargo.toml")).unwrap();

        let index_async = rt.block_on(FileIndex::new(&file)).unwrap();
        let index_sync = FileIndex::new_sync(&rt.block_on(file.into_std())).unwrap();

        assert_eq!(index_async, index_sync);
    }
}
