use anyhow::Result;
use std::sync::Arc;

pub(crate) struct Shard {
    pub(crate) id: usize,
    pub(crate) data: memmap2::Mmap,
    pub(crate) start: u64,
}

impl Shard {
    pub(crate) fn translate_inner_data_range(&self, start: u64, end: u64) -> (u64, u64) {
        (start - self.start, end - self.start)
    }

    pub fn get_shard_line(self: &Arc<Self>, start: u64, end: u64) -> Result<ShardStr> {
        let str = &self.data[start as usize..end as usize];
        ShardStr::new(self.clone(), str.as_ptr(), (end - start) as usize)
    }
}

/// A line that comes from a shard.
/// The shard will not be dropped until all of its lines have been dropped.
/// This structure avoids cloning unnecessarily.
pub struct ShardStr {
    pub(crate) _origin: Arc<Shard>,
    // This data point to the ref-counted arc
    pub(crate) ptr: *const u8,
    pub(crate) len: usize,
}

impl ShardStr {
    pub(crate) fn new(origin: Arc<Shard>, ptr: *const u8, len: usize) -> Result<Self> {
        // Safety: the ptr came from an immutable slice
        let slice = unsafe { std::slice::from_raw_parts(ptr, len) };
        // Check if it is utf8 for later
        std::str::from_utf8(slice)?;
        Ok(Self {
            _origin: origin,
            ptr,
            len,
        })
    }

    pub fn as_bytes(&self) -> &[u8] {
        // Safety: the ptr came from an immutable slice
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }

    pub fn as_str(&self) -> &str {
        // Safety: We have checked in new
        unsafe { std::str::from_utf8_unchecked(self.as_bytes()) }
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
        self.as_str()
    }
}

impl std::convert::AsRef<str> for ShardStr {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl std::fmt::Display for ShardStr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self)
    }
}
