use std::{borrow::Cow, sync::Arc};

use anyhow::Result;
use quick_cache::sync::Cache;
use tokio::fs::File;

use self::index::FileIndex;

mod index;

struct Shard {
    id: usize,
    data: memmap2::Mmap,
    start: u64,
}

/// A line that comes from a shard.
/// The shard will not be dropped until all of its lines have been dropped.
/// This structure avoids cloning unnecessarily.
pub struct ShardStr {
    _origin: Arc<Shard>,
    // This data point to the ref-counted arc
    ptr: *const u8,
    len: usize,
}

impl ShardStr {
    fn new(origin: Arc<Shard>, ptr: *const u8, len: usize) -> Result<Self> {
        // Safety: the ptr came from an immutable slice
        let slice = unsafe { std::slice::from_raw_parts(ptr, len) };
        // This does the checking
        std::str::from_utf8(slice)?;
        Ok(Self { _origin: origin, ptr, len })
    }
}

impl std::borrow::Borrow<str> for ShardStr {
    fn borrow(&self) -> &str {
        self
    }
}

impl std::ops::Deref for ShardStr {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        unsafe {
            // Safety: the ptr came from an immutable slice
            let slice = std::slice::from_raw_parts(self.ptr, self.len);
            // Safety: we have already done our checking
            std::str::from_utf8_unchecked(slice)
        }
    }
}

impl std::fmt::Display for ShardStr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self)
    }
}

impl Shard {
    fn translate_inner_data_range(&self, start: u64, end: u64) -> (u64, u64) {
        (start - self.start, end - self.start)
    }

    pub fn get_shard_line(self: &Arc<Self>, start: u64, end: u64) -> Result<ShardStr> {
        let str = &self.data[start as usize..end as usize];
        ShardStr::new(self.clone(), str.as_ptr(), (end - start) as usize)
    }

    fn get_data(&self, start: u64, end: u64) -> Cow<str> {
        String::from_utf8_lossy(&self.data[start as usize..end as usize])
    }
}

pub struct ShardedFile {
    file: File,
    index: index::FileIndex,
    shards: Cache<usize, Arc<Shard>>,
}

impl ShardedFile {
    pub async fn new(file: File, shard_count: usize) -> Result<Self> {
        let len = file.metadata().await?.len();
        let index = FileIndex::new(&file, len).await?;
        Ok(Self {
            file,
            index,
            shards: Cache::new(shard_count),
        })
    }

    pub fn line_count(&self) -> usize {
        self.index.line_count()
    }

    fn get_shard_of_line(&self, line_number: usize) -> Result<Arc<Shard>> {
        let (shard_id, range) = self.index.data_range_of_line(line_number).unwrap();
        self.shards.get_or_insert_with(&shard_id, || {
            let data = unsafe {
                memmap2::MmapOptions::new()
                    .offset(range.start)
                    .len((range.end - range.start) as usize)
                    .map(&self.file)?
            };
            Ok(Arc::new(Shard {
                id: shard_id,
                data,
                start: range.start,
            }))
        })
    }

    fn get_shard(&self, shard_id: usize) -> Result<Arc<Shard>> {
        let range = self.index.data_range_of_shard(shard_id).unwrap();
        self.shards.get_or_insert_with(&shard_id, || {
            let data = unsafe {
                memmap2::MmapOptions::new()
                    .offset(range.start)
                    .len((range.end - range.start) as usize)
                    .map(&self.file)?
            };
            Ok(Arc::new(Shard {
                id: shard_id,
                data,
                start: range.start,
            }))
        })
    }

    pub fn get_line(&self, line_number: usize) -> Result<ShardStr> {
        assert!(line_number < self.line_count());
        let shard = self.get_shard_of_line(line_number)?;
        
        // prefetch shards
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

    fn get_line_from_shard(&self, shard: &Arc<Shard>, line_number: usize) -> Result<ShardStr> {
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
        let file = rt.block_on(ShardedFile::new(file, 25)).unwrap();
        dbg!(file.line_count());

        for i in 0..file.line_count() {
            eprintln!("{}\t{}", i + 1, file.get_line(i).unwrap());
        }
    }
}
