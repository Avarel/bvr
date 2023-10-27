use std::{borrow::Cow, num::NonZeroUsize, ops::Range, sync::Arc};

use anyhow::Result;
use quick_cache::sync::Cache;
use tokio::{fs::File, sync::mpsc::Receiver};

type FileIndex = Vec<u64>;

struct IndexingTask {
    sx: tokio::sync::mpsc::Sender<u64>,
    data: memmap2::Mmap,
    start: u64,
}

impl IndexingTask {
    async fn new(file: &File, start: u64, end: u64) -> Result<(Self, Receiver<u64>)> {
        let data = unsafe {
            memmap2::MmapOptions::new()
                .offset(start)
                .len((end - start) as usize)
                .map(file)?
        };
        let (sx, rx) = tokio::sync::mpsc::channel(1 << 10);
        Ok((Self { sx, data, start }, rx))
    }

    async fn worker(self) -> Result<()> {
        for i in memchr::memchr_iter(b'\n', &self.data) {
            self.sx.send(self.start + i as u64).await?;
        }

        Ok(())
    }
}

struct Shard {
    data: memmap2::Mmap,
    start: u64,
}

impl Shard {
    fn translate_inner_data_range(&self, start: u64, end: u64) -> (u64, u64) {
        (start - self.start, end - self.start)
    }

    fn get_data(&self, start: u64, end: u64) -> Cow<str> {
        String::from_utf8_lossy(&self.data[start as usize..end as usize])
    }
}

struct Shards {
    shards: Cache<usize, Arc<Shard>>,
    lines_per_shard: usize,
}

impl Shards {
    fn new(shard_count: usize, lines_per_shard: usize) -> Self {
        Self {
            shards: Cache::new(shard_count),
            lines_per_shard,
        }
    }

    fn get_shard_index(&self, line_number: usize) -> usize {
        line_number / self.lines_per_shard
    }

    fn get_shard_line_range(&self, shard_index: usize, line_count: usize) -> (usize, usize) {
        let begin = shard_index * self.lines_per_shard;
        let end = (shard_index + 1) * self.lines_per_shard;
        (begin, end.min(line_count))
    }

    fn get_shard_data_range(&self, f: &ShardedFile, shard_index: usize) -> (u64, u64) {
        let (start, end) = self.get_shard_line_range(shard_index, f.line_count());
        (f.index[start], f.index[end])
    }

    fn get_shard(&self, f: &ShardedFile, shard_index: usize) -> Result<Arc<Shard>> {
        self.shards.get_or_insert_with(&shard_index, || {
            let (start, end) = self.get_shard_data_range(f, shard_index);
            let data = unsafe {
                memmap2::MmapOptions::new()
                    .offset(start)
                    .len((end - start) as usize)
                    .map(&f.file)?
            };
            Ok(Arc::new(Shard { data, start }))
        })
    }
}

pub struct ShardedFile {
    file: File,
    index: FileIndex,
    shards: Shards,
}

impl ShardedFile {
    pub async fn new(file: File, lines_per_shard: usize, shard_count: usize) -> Result<Self> {
        let len = file.metadata().await?.len();
        let index = Self::index_file(&file, len).await?;
        Ok(Self {
            file,
            index,
            shards: Shards::new(shard_count, lines_per_shard)
        })
    }

    pub fn line_count(&self) -> usize {
        self.index.len() - 1
    }

    fn get_shard(&self, shard_index: usize) -> Result<Arc<Shard>> {
        self.shards.get_shard(self, shard_index)
    }

    fn last_shard_index(&self) -> usize {
        self.shards.get_shard_index(self.line_count())
    }

    pub fn get_line(&self, line_number: usize) -> Result<String> {
        assert!(line_number < self.line_count());
        let shard_index = self.shards.get_shard_index(line_number);
        let shard = self.get_shard(shard_index)?;
        self.get_line_from_shard(&shard, line_number)
    }

    pub async fn get_line_async(self: &Arc<Self>, line_number: usize) -> Result<String> {
        assert!(line_number < self.line_count());
        let shard_index = self.shards.get_shard_index(line_number);

        if shard_index > 0 {
            let arc = self.clone();
            tokio::task::spawn_blocking(move || arc.get_shard(shard_index - 1).ok());
        }
        if shard_index < self.last_shard_index() {
            let arc = self.clone();
            tokio::task::spawn_blocking(move || arc.get_shard(shard_index + 1).ok());
        }

        let shard = self.get_shard(shard_index)?;
        self.get_line_from_shard(&shard, line_number)
    }

    fn get_line_from_shard(&self, shard: &Shard, line_number: usize) -> Result<String> {
        let (start, end) =
            shard.translate_inner_data_range(self.index[line_number], self.index[line_number + 1]);
        let start = if line_number == 0 { 0 } else { start + 1 };
        Ok(shard.get_data(start, end).into_owned())
    }

    async fn index_file(file: &File, len: u64) -> Result<FileIndex> {
        let (sx, mut rx) = tokio::sync::mpsc::channel(10);

        let file = file.try_clone().await?;

        let spawner = tokio::task::spawn(async move {
            const SIZE: u64 = 1 << 20;
            let mut curr = 0;

            while curr < len {
                let end = (curr + SIZE).min(len);
                let (task, task_rx) = IndexingTask::new(&file, curr, end).await?;
                sx.send(task_rx).await?;
                tokio::task::spawn(task.worker());

                curr = end;
            }

            Ok::<(), anyhow::Error>(())
        });

        let mut result = FileIndex::new();
        result.push(0);

        while let Some(mut task_rx) = rx.recv().await {
            while let Some(v) = task_rx.recv().await {
                result.push(v);
            }
        }

        result.push(len);

        spawner.await??;

        Ok(result)
    }
}

#[cfg(test)]
mod test {
    use std::sync::Arc;

    use crate::file::ShardedFile;

    #[test]
    fn what() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let file = rt.block_on(tokio::fs::File::open("./Cargo.toml")).unwrap();
        let file = rt.block_on(ShardedFile::new(file, 100000, 25)).unwrap();
        let file = Arc::new(file);

        dbg!(file.line_count());

        for i in 0..file.line_count() {
            eprintln!("{}\t{}", i + 1, file.get_line(i).unwrap());
        }

        // for i in 0..file.line_count() {
        //     eprintln!("{}\t{}", i + 1, rt.block_on(file.get_line_async(i)).unwrap());
        // }
    }
}
