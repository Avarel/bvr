use anyhow::Result;
use std::{cell::Cell, ops::Range, os::fd::AsRawFd, ptr::NonNull, rc::Rc};

pub struct Shard {
    id: usize,
    data: memmap2::Mmap,
    start: u64,
}

impl Shard {
    pub fn new<F: AsRawFd>(id: usize, range: Range<u64>, file: &F) -> Result<Self> {
        let data = unsafe {
            memmap2::MmapOptions::new()
                .offset(range.start)
                .len((range.end - range.start) as usize)
                .map(file)?
        };
        data.advise(memmap2::Advice::WillNeed)?;
        Ok(Self {
            id,
            data,
            start: range.start,
        })
    }

    pub fn id(&self) -> usize {
        self.id
    }

    pub fn translate_inner_data_range(&self, start: u64, end: u64) -> (u64, u64) {
        (start - self.start, end - self.start)
    }

    pub fn get_shard_line(self: &Rc<Self>, start: u64, end: u64) -> Result<ShardStr> {
        let str = &self.data[start as usize..end as usize];
        ShardStr::new(
            self.clone(),
            // Safety: this ptr came from a slice
            unsafe { NonNull::new(str.as_ptr() as *mut _).unwrap_unchecked() },
            (end - start) as usize,
        )
    }
}

/// A line that comes from a shard.
/// The shard will not be dropped until all of its lines have been dropped, essentially
/// pinning the shard.
///
/// This structure avoids cloning unnecessarily.
pub struct ShardStr {
    _origin: Rc<Shard>,
    // This data point to the ref-counted arc
    repr: Cell<ShardStrRepr>,
}

#[derive(Clone, Copy)]
enum ShardStrRepr {
    Unchecked(FatPtr),
    Borrowed(FatPtr),
    Error,
}

#[derive(Clone, Copy)]
struct FatPtr {
    ptr: NonNull<u8>,
    len: usize,
}

impl FatPtr {
    fn as_bytes(&self) -> &'static [u8] {
        // Safety: this came from origin, we borrow for 'static but the lifetime
        // is tied to self in as_str()
        unsafe { std::slice::from_raw_parts(self.ptr.as_ptr(), self.len) }
    }
}

impl ShardStr {
    const ERROR_STR: &str = "!!! line is not utf-8 !!!";

    /// Constructs a string that lives inside
    /// # Contract
    /// 1. The provided pointer must point to data that lives inside the ref-counted [Shard].
    /// 2. The length must be valid.
    fn new(origin: Rc<Shard>, ptr: NonNull<u8>, len: usize) -> Result<Self> {
        Ok(Self {
            _origin: origin,
            repr: Cell::new(ShardStrRepr::Unchecked(FatPtr { ptr, len })),
        })
    }

    pub fn as_str(&self) -> &str {
        match self.repr.get() {
            ShardStrRepr::Unchecked(repr) => match std::str::from_utf8(repr.as_bytes()) {
                Ok(s) => {
                    self.repr.replace(ShardStrRepr::Borrowed(repr));
                    s
                }
                Err(_) => {
                    self.repr.replace(ShardStrRepr::Error);
                    Self::ERROR_STR
                }
            },
            ShardStrRepr::Borrowed(repr) => unsafe {
                // Safety: we have already checked this
                std::str::from_utf8_unchecked(repr.as_bytes())
            },
            ShardStrRepr::Error => Self::ERROR_STR,
        }
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
