use std::{borrow::Cow, ops::Range, ptr::NonNull, rc::Rc};
use crate::Mmappable;

pub struct Shard {
    id: usize,
    start: u64,
    data: memmap2::Mmap,
}

impl Shard {
    pub fn map_file<F: Mmappable>(id: usize, range: Range<u64>, file: &F) -> Self {
        let data = unsafe {
            memmap2::MmapOptions::new()
                .offset(range.start)
                .len((range.end - range.start) as usize)
                .map(file).expect("mmap should succeed")
        };
        #[cfg(unix)]
        data.advise(memmap2::Advice::WillNeed).ok();
        Self::new(id, range.start, data)
    }

    pub fn new(id: usize, start: u64, data: memmap2::Mmap) -> Self {
        Self {
            id,
            data,
            start,
        }
    }

    pub fn id(&self) -> usize {
        self.id
    }

    pub fn as_slice(&self) -> &[u8] {
        &self
    }

    pub fn translate_inner_data_index(&self, start: u64) -> u64 {
        start - self.start
    }

    pub fn translate_inner_data_range(&self, start: u64, end: u64) -> (u64, u64) {
        (self.translate_inner_data_index(start), self.translate_inner_data_index(end))
    }

    pub fn get_shard_line(self: &Rc<Self>, start: u64, end: u64) -> ShardStr {
        let data = &self.data[start as usize..end as usize];
        // Safety: The length is computed by a (assumed to be correct)
        //         index. It is undefined behavior if the file changes
        //         in a non-appending way after the index is created.
        ShardStr::new(self.clone(), data)
    }
}

impl std::ops::Deref for Shard {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.data
    }
}

/// Line string that comes from a [Shard].
///
/// If the [ShardStr] borrows from the shard, the shard will not be dropped until
/// all of its pin is dropped.
///
/// This structure avoids cloning unnecessarily.
pub struct ShardStr(ShardStrRepr);

/// Internal representation of [ShardStr].
enum ShardStrRepr {
    Borrowed {
        // This field pins the shard so its data does not get munmap'd and remains valid.
        _pin: Rc<Shard>,
        // This data point to the ref-counted `_pin` field.
        // Maybe if polonius supports self-referential slices one day, this
        // spicy unsafe code can be dropped.
        ptr: NonNull<u8>,
        len: usize,
    },
    Owned(String),
}

impl ShardStr {
    /// Constructs a string that might borrows data from a [Shard]. If the data
    /// is invalid utf-8, it will be converted into an owned [String] using `String::from_utf8_lossy`.
    ///
    /// # Safety
    /// 1. The provided slice must point to data that lives inside the ref-counted [Shard].
    /// 2. The length must encompass a valid range of data inside the [Shard].
    fn new<'rc>(origin: Rc<Shard>, data: &'rc [u8]) -> Self {
        // Safety: This ptr came from a slice that we prevent from
        //         being dropped by having it inside a ref counter
        match String::from_utf8_lossy(data) {
            Cow::Borrowed(_) => Self(ShardStrRepr::Borrowed {
                _pin: origin,
                ptr: unsafe { NonNull::new(data.as_ptr() as *mut _).unwrap_unchecked() },
                len: data.len(),
            }),
            Cow::Owned(s) => Self::new_owned(s),
        }
    }

    /// Constructs a string that owns its data.
    pub fn new_owned(s: String) -> Self {
        Self(ShardStrRepr::Owned(s))
    }

    /// Returns a byte slice of this [ShardStr]'s components.
    pub fn as_bytes(&self) -> &[u8] {
        // Safety: We have already checked in the constructor.
        match &self.0 {
            ShardStrRepr::Borrowed { _pin, ptr, len } => unsafe {
                std::slice::from_raw_parts(ptr.as_ptr(), *len)
            },
            ShardStrRepr::Owned(s) => s.as_bytes(),
        }
    }

    /// Extract a [str] slice backed by the pinned shard data or owned data.
    pub fn as_str(&self) -> &str {
        // Safety: we already did utf-8 checking
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

impl std::fmt::Debug for ShardStr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&**self, f)
    }
}
