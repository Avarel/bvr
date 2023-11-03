pub mod shard;

use std::sync::Arc;

use anyhow::Result;
use quick_cache::sync::Cache;
use tokio::fs::File;

use self::shard::ShardStr;
use crate::index::{sync::AsyncIndex, CompleteIndex, FileIndex};

pub struct ShardedFile<Idx> {
    file: File,
    index: Idx,
    shards: Cache<usize, Arc<shard::Shard>>,
}

impl ShardedFile<AsyncIndex> {
    pub async fn new(file: File, shard_count: usize) -> Result<Self> {
        let (index, indexer) = AsyncIndex::new();
        tokio::spawn(indexer.index(file.try_clone().await?));

        Ok(Self {
            file,
            index,
            shards: Cache::new(shard_count),
        })
    }

    pub fn try_finalize(&mut self) -> bool {
        self.index.try_finalize()
    }

    pub fn progress(&self) -> f64 {
        self.index.progress()
    }
}

impl ShardedFile<CompleteIndex> {
    #[cfg(test)]
    async fn new_complete(file: File, shard_count: usize) -> Result<Self> {
        let (mut index, indexer) = AsyncIndex::new();
        indexer.index(file.try_clone().await?).await?;
        assert!(index.try_finalize());

        Ok(Self {
            file,
            index: index.unwrap(),
            shards: Cache::new(shard_count),
        })
    }
}

impl<Idx: FileIndex> ShardedFile<Idx> {
    pub fn line_count(&self) -> usize {
        self.index.line_count()
    }

    fn get_shard_of_line(&self, line_number: usize) -> Result<Arc<shard::Shard>> {
        self.get_shard(self.index.shard_of_line(line_number).unwrap())
    }

    fn get_shard(&self, shard_id: usize) -> Result<Arc<shard::Shard>> {
        let range = self.index.data_range_of_shard(shard_id).unwrap();
        self.shards.get_or_insert_with(&shard_id, || {
            let data = unsafe {
                memmap2::MmapOptions::new()
                    .offset(range.start)
                    .len((range.end - range.start) as usize)
                    .map(&self.file)?
            };
            Ok(Arc::new(shard::Shard {
                id: shard_id,
                data,
                start: range.start,
            }))
        })
    }

    pub fn get_line(&self, line_number: usize) -> Result<ShardStr> {
        assert!(line_number <= self.line_count());
        let shard = self.get_shard_of_line(line_number)?;

        if self.shards.capacity() > 3 {
            let shard_id = shard.id;
            if shard_id > 0 {
                self.get_shard(shard_id - 1).ok();
            }
            if shard_id < self.index.shard_count() - 1 {
                self.get_shard(shard_id + 1).ok();
            }
        }

        self.get_line_from_shard(&shard, line_number)
    }

    fn get_line_from_shard(
        &self,
        shard: &Arc<shard::Shard>,
        line_number: usize,
    ) -> Result<shard::ShardStr> {
        let (start, end) = shard.translate_inner_data_range(
            self.index.start_of_line(line_number),
            self.index.start_of_line(line_number + 1),
        );
        let start = if line_number == 0 { 0 } else { start + 1 };
        shard.get_shard_line(start, end)
    }
}

#[cfg(test)]
mod test {
    use crate::file::ShardedFile;

    #[test]
    fn what() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let file = rt.block_on(tokio::fs::File::open("./Cargo.toml")).unwrap();
        let file = rt.block_on(ShardedFile::new_complete(file, 25)).unwrap();
        dbg!(file.line_count());

        for i in 0..=file.line_count() {
            eprintln!("{}\t{}", i + 1, file.get_line(i).unwrap());
        }
    }
}
