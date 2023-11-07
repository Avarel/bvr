pub mod shard;

use std::{rc::Rc, num::NonZeroUsize};

use anyhow::Result;
use lru::LruCache;
use tokio::fs::File;

use self::shard::{ShardStr, Shard};
use crate::index::{sync::AsyncIndex, CompleteIndex, FileIndex};

pub struct ShardedFile<Idx> {
    file: File,
    index: Idx,
    shards: LruCache<usize, Rc<Shard>>,
}

impl ShardedFile<AsyncIndex> {
    pub async fn read_file(file: File, shard_count: usize) -> Result<Self> {
        let (index, indexer) = AsyncIndex::new();
        tokio::spawn(indexer.index(file.try_clone().await?));

        Ok(Self {
            file,
            index,
            shards: LruCache::new(NonZeroUsize::new(shard_count).unwrap()),
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
    async fn read_file(file: File, shard_count: usize) -> Result<Self> {
        let (mut index, indexer) = AsyncIndex::new();
        indexer.index(file.try_clone().await?).await?;
        assert!(index.try_finalize());

        Ok(Self {
            file,
            index: index.unwrap(),
            shards: LruCache::new(NonZeroUsize::new(shard_count).unwrap()),
        })
    }
}

impl<Idx: FileIndex> ShardedFile<Idx> {
    pub fn line_count(&self) -> usize {
        self.index.line_count()
    }

    fn get_shard_of_line(&mut self, line_number: usize) -> Result<Rc<Shard>> {
        self.get_shard(self.index.shard_of_line(line_number).unwrap())
    }

    fn get_shard(&mut self, shard_id: usize) -> Result<Rc<Shard>> {
        let range = self.index.data_range_of_shard(shard_id).unwrap();
        self.shards.try_get_or_insert(shard_id, || {
            Ok(Rc::new(Shard::new(shard_id, range, &self.file)?))
        }).cloned()
    }

    pub fn get_line(&mut self, line_number: usize) -> Result<ShardStr> {
        assert!(line_number <= self.line_count());
        let shard = self.get_shard_of_line(line_number)?;

        if usize::from(self.shards.cap()) > 3 {
            let shard_id = shard.id();
            if shard_id > 0 {
                self.get_shard(shard_id - 1).ok();
            }
            if shard_id < self.index.shard_count() - 1 {
                self.get_shard(shard_id + 1).ok();
            }
        }

        Ok(self.get_line_from_shard(&shard, line_number))
    }

    fn get_line_from_shard(
        &self,
        shard: &Rc<Shard>,
        line_number: usize,
    ) -> ShardStr {
        let (start, end) = shard.translate_inner_data_range(
            self.index.start_of_line(line_number),
            self.index.start_of_line(line_number + 1),
        );
        // Trim the newline
        let start = if line_number == 0 { 0 } else { start + 1 };
        shard.get_shard_line(start, end)
    }
}

#[cfg(test)]
mod test {
    use crate::{file::ShardedFile, index::CompleteIndex};

    #[test]
    fn what() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let file = rt.block_on(tokio::fs::File::open("./Cargo.toml")).unwrap();
        let mut file = rt.block_on(ShardedFile::<CompleteIndex>::read_file(file, 25)).unwrap();
        dbg!(file.line_count());

        for i in 0..=file.line_count() {
            eprintln!("{}\t{}", i + 1, file.get_line(i).unwrap());
        }
    }
}
