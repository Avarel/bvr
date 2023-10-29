use core::panic;
use std::cmp::Ordering;
use std::ops::Range;

use anyhow::Result;
use tokio::fs::File;
use tokio::sync::mpsc::Receiver;

pub(super) struct IncompleteIndex {
    index: FileIndex,
    last_line: usize,
    data_size: u64,
    shard_i: usize,
    finished: bool,
}

impl IncompleteIndex {
    /// Constant to determine how much data is stored in each shard.
    /// Note that the shard likely contains more than the threshold,
    /// this is just a cutoff.
    const SHARD_DATA_THRESHOLD: u64 = 0;

    /// How much data of the file should each indexing task handle?
    const INDEXING_VIEW_SIZE: u64 = 1 << 20;

    fn new() -> Self {
        Self {
            index: FileIndex::empty(),
            last_line: 0,
            data_size: 0,
            shard_i: 0,
            finished: false,
        }
    }

    fn simple_index(&mut self, file: &File, len: u64) -> Result<()> {
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

        Ok(())
    }

    fn push_line_data(&mut self, line_data: u64) {
        let line_number = self.index.line_index.len();
        self.index.line_index.push(line_data);

        self.data_size += line_data;
        if self.data_size > Self::SHARD_DATA_THRESHOLD {
            self.index
                .line_to_shard
                .insert(self.last_line..line_number, self.shard_i);

            self.last_line = line_number;
            self.data_size = 0;
            self.shard_i += 1;
        }
    }
    fn finalize(&mut self, len: u64) {
        self.index.line_index.push(len);
        // In case the shard boundary did not end on the very last line we iterated through
        if self.last_line + 1 < self.index.line_index.len() {
            self.index.line_to_shard.insert(
                self.last_line..self.index.line_index.len() - 1,
                self.shard_i,
            );
        }
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

    async fn index(&self, file: &File, len: u64) -> Result<()> {
        // Build line & shard index
        let (sx, mut rx) = tokio::sync::mpsc::channel(10);

        let file = file.try_clone().await?;

        // Indexing worker
        let spawner = tokio::task::spawn(async move {
            let mut curr = 0;

            while curr < len {
                let end = (curr + IncompleteIndex::INDEXING_VIEW_SIZE).min(len);
                let (task, task_rx) = IndexingTask::new(&file, curr, end)?;
                sx.send(task_rx).await?;
                tokio::task::spawn(task.worker());

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
        self.write(|z| z.finalize(len)).await;

        spawner.await?
    }

    async fn write<F>(&self, cb: F)
    where
        F: FnOnce(&mut IncompleteIndex),
    {
        match self {
            AsyncIndex::Incomplete(inner) => cb(&mut *inner.lock().await),
            AsyncIndex::Complete(_) => panic!("Cannot edit, its already finished!"),
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

    pub async fn line_range_of_line(&self, line_number: usize) -> Option<(usize, Range<usize>)> {
        self.read(|index| index.line_range_of_line(line_number))
            .await
    }

    pub async fn translate_data_from_line_range(&self, line_range: Range<usize>) -> Range<u64> {
        self.read(|index| index.translate_data_from_line_range(line_range))
            .await
    }

    pub async fn data_range_of_line(&self, line_number: usize) -> Option<(usize, Range<u64>)> {
        self.read(|index| index.data_range_of_line(line_number))
            .await
    }

    pub async fn line_range_of_shard(&self, shard_id: usize) -> Option<Range<usize>> {
        self.read(|index| index.line_range_of_shard(shard_id)).await
    }

    pub async fn data_range_of_shard(&self, shard_id: usize) -> Option<Range<u64>> {
        self.read(|index| index.data_range_of_shard(shard_id)).await
    }
}

struct IndexingTask {
    sx: tokio::sync::mpsc::Sender<u64>,
    data: memmap2::Mmap,
    start: u64,
}

// 2778040496ns
// 366257645ns
// 401660998ns

impl IndexingTask {
    const LINES_PER_MB: usize = 1 << 13;

    fn new(file: &File, start: u64, end: u64) -> Result<(Self, Receiver<u64>)> {
        let data = unsafe {
            memmap2::MmapOptions::new()
                .offset(start)
                .len((end - start) as usize)
                .map(file)?
        };
        data.advise(memmap2::Advice::Sequential)?;
        let (sx, rx) = tokio::sync::mpsc::channel(Self::LINES_PER_MB);
        Ok((Self { sx, data, start }, rx))
    }

    async fn worker(self) -> Result<()> {
        for i in memchr::memchr_iter(b'\n', &self.data) {
            self.sx.send(self.start + i as u64).await?;
        }

        Ok(())
    }
}

#[derive(Debug)]
struct RangePartition {
    inner: Vec<Range<usize>>
}

impl RangePartition {
    fn new() -> Self {
        Self {
            inner: Vec::new()
        }
    }

    fn len(&self) -> usize {
        self.inner.len()
    }

    fn insert(&mut self, key: Range<usize>, value: usize) {
        debug_assert!(self.inner.last().map(|Range { end, .. }| *end == key.start).unwrap_or(true));
        assert_eq!(value, self.inner.len());
        self.inner.push(key);
    }

    fn reverse_lookup(&self, value: usize) -> Option<Range<usize>> {
        self.inner.get(value).cloned()
    }

    fn get(&self, key: usize) -> Option<usize> {
        self.inner.binary_search_by(|probe| {
            if probe.start > key {
                Ordering::Greater
            } else if probe.end <= key {
                Ordering::Less
            } else {
                Ordering::Equal
            }
        }).ok()
    }

    fn get_key_value(&self, key: usize) -> Option<(usize, Range<usize>)> {
        self.get(key).map(|idx| (idx, self.inner[idx].clone()))
    }
}

pub(super) struct FileIndex {
    /// Store the byte location of the start of the indexed line
    line_index: Vec<u64>,
    /// Allow queries from line number in a line range to shard
    line_to_shard: RangePartition,
}

impl FileIndex {
    fn empty() -> Self {
        Self {
            line_index: vec![0],
            line_to_shard: RangePartition::new(),
        }
    }

    pub(super) async fn new(file: &File, len: u64) -> Result<Self> {
        let mut result = AsyncIndex::new();
        result.index(file, len).await?;
        assert!(result.try_finalize());
        Ok(result.into_inner())
        // let mut result = IncompleteIndex::new();
        // result.simple_index(file, len)?;
        // Ok(result.finish())
    }

    pub fn line_count(&self) -> usize {
        self.line_index.len() - 2
    }

    pub fn shard_count(&self) -> usize {
        self.line_to_shard.len()
    }

    pub fn start_of_line(&self, line_number: usize) -> u64 {
        self.line_index[line_number]
    }

    pub fn shard_of_line(&self, line_number: usize) -> Option<usize> {
        self.line_to_shard.get(line_number)
    }

    pub fn line_range_of_line(&self, line_number: usize) -> Option<(usize, Range<usize>)> {
        self.line_to_shard
            .get_key_value(line_number)
            .map(|(shard_id, range)| (shard_id, range.clone()))
    }

    pub fn translate_data_from_line_range(&self, line_range: Range<usize>) -> Range<u64> {
        self.start_of_line(line_range.start)..self.start_of_line(line_range.end)
    }

    pub fn data_range_of_line(&self, line_number: usize) -> Option<(usize, Range<u64>)> {
        self.line_range_of_line(line_number)
            .map(|(shard_id, range)| (shard_id, self.translate_data_from_line_range(range)))
    }

    pub fn line_range_of_shard(&self, shard_id: usize) -> Option<Range<usize>> {
        self.line_to_shard.reverse_lookup(shard_id)
    }

    pub fn data_range_of_shard(&self, shard_id: usize) -> Option<Range<u64>> {
        self.line_range_of_shard(shard_id)
            .map(|range| self.translate_data_from_line_range(range))
    }
}
