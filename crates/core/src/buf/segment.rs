use crate::Result;
use memmap2::{Mmap, MmapMut};
use std::{borrow::Cow, ops::Range, ptr::NonNull, sync::Arc};

#[cfg(unix)]
pub(crate) use std::os::fd::AsRawFd as Mmappable;
#[cfg(windows)]
pub(crate) use std::os::windows::io::AsRawHandle as Mmappable;

pub struct SegmentRaw<Buf> {
    range: Range<u64>,
    data: Buf,
}

pub type SegmentMut = SegmentRaw<MmapMut>;
pub type Segment = SegmentRaw<Mmap>;

impl<Buf> SegmentRaw<Buf>
where
    Buf: AsRef<[u8]>,
{
    pub const TODO_REMOVE_SIZE: u64 = 1 << 20;

    #[inline]
    pub fn start(&self) -> u64 {
        self.range.start
    }

    #[inline]
    pub fn translate_inner_data_index(&self, start: u64) -> u64 {
        debug_assert!(self.range.start <= start);
        // TODO: make this better... i don't like that its <=
        //       but technically its fine as long as start
        //       is the end of the buffer
        debug_assert!(start <= self.range.end);
        start - self.range.start
    }

    #[inline]
    pub fn translate_inner_data_range(&self, start: u64, end: u64) -> Range<u64> {
        self.translate_inner_data_index(start)..self.translate_inner_data_index(end)
    }
}

impl<Buf> std::ops::Deref for SegmentRaw<Buf>
where
    Buf: std::ops::Deref<Target = [u8]>,
{
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.data
    }
}

impl<Buf> std::ops::DerefMut for SegmentRaw<Buf>
where
    Buf: std::ops::DerefMut<Target = [u8]>,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.data
    }
}

impl SegmentMut {
    pub(crate) fn new(start: u64, len: u64) -> Result<Self> {
        let data = memmap2::MmapOptions::new()
            .len(len as usize)
            .map_anon()?;
        #[cfg(unix)]
        data.advise(memmap2::Advice::Sequential)?;
        Ok(Self {
            data,
            range: start..start + len,
        })
    }

    pub fn into_read_only(self) -> Result<Segment> {
        Ok(Segment {
            data: self.data.make_read_only()?,
            range: self.range,
        })
    }
}

impl Segment {
    pub(crate) fn map_file<F: Mmappable>(range: Range<u64>, file: &F) -> Result<Self> {
        let size = range.end - range.start;
        // debug_assert!(size <= Self::MAX_SIZE);
        let data = unsafe {
            memmap2::MmapOptions::new()
                .offset(range.start)
                .len(size as usize)
                .map(file)?
        };
        #[cfg(unix)]
        data.advise(memmap2::Advice::WillNeed)?;
        Ok(Self { data, range })
    }

    #[inline]
    pub fn get_line(self: &Arc<Self>, range: Range<u64>) -> SegStr {
        SegStr::from_bytes(self.get_bytes(range))
    }

    #[inline]
    pub fn get_bytes(self: &Arc<Self>, range: Range<u64>) -> SegBytes {
        SegBytes::new_borrow(self.clone(), range)
    }
}

/// Line buffer that comes from a [Segment].
///
/// If the [SegSlice] borrows from the segment, the segment will not be dropped until
/// all of its referents is dropped.
///
/// This structure avoids cloning unnecessarily.
pub struct SegBytes(SegBytesRepr);

/// Internal representation of [SegSlice].
enum SegBytesRepr {
    Borrowed {
        // This field refs the segment so its data does not get munmap'd and remains valid.
        _ref: Arc<Segment>,
        // This data point to the ref-counted `_pin` field.
        // Maybe if polonius supports self-referential slices one day, this
        // spicy unsafe code can be dropped.
        ptr: NonNull<u8>,
        len: usize,
    },
    Owned(Vec<u8>),
}

impl SegBytes {
    /// Constructs a string that might borrows data from a [Segment]. If the data
    /// is invalid utf-8, it will be converted into an owned [String] using `String::from_utf8_lossy`.
    ///
    /// # Safety
    ///
    /// 1. The provided slice must point to data that lives inside the ref-counted [Segment].
    /// 2. The length must encompass a valid range of data inside the [Segment].
    fn new_borrow(origin: Arc<Segment>, range: Range<u64>) -> Self {
        // Safety: This ptr came from a slice that we prevent from
        //         being dropped by having it inside a ref counter
        // Safety: The length is computed by a (assumed to be correct)
        //         index. It is undefined behavior if the file changes
        //         in a non-appending way after the index is created.
        let data = &origin.data[range.start as usize..range.end as usize];
        Self(SegBytesRepr::Borrowed {
            ptr: unsafe { NonNull::new(data.as_ptr().cast_mut()).unwrap_unchecked() },
            len: data.len(),
            _ref: origin,
        })
    }

    /// Constructs a string that owns its data.
    #[inline]
    pub fn new_owned(s: Vec<u8>) -> Self {
        Self(SegBytesRepr::Owned(s))
    }

    /// Returns a byte slice of this [SegBytes]'s components.
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        // Safety: We have already checked in the constructor.
        match &self.0 {
            SegBytesRepr::Borrowed { ptr, len, .. } => unsafe {
                std::slice::from_raw_parts(ptr.as_ptr(), *len)
            },
            SegBytesRepr::Owned(s) => s.as_slice(),
        }
    }
}

impl std::borrow::Borrow<[u8]> for SegBytes {
    #[inline]
    fn borrow(&self) -> &[u8] {
        self
    }
}

impl std::ops::Deref for SegBytes {
    type Target = [u8];

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.as_bytes()
    }
}

impl std::convert::AsRef<[u8]> for SegBytes {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

/// Line string that comes from a [Segment].
///
/// If the [SegStr] borrows from the segment, the segment will not be dropped until
/// all of its referents is dropped.
///
/// This structure avoids cloning unnecessarily.
#[derive(Clone)]
pub struct SegStr(SegStrRepr);

/// Internal representation of [SegStr].
#[derive(Clone)]
enum SegStrRepr {
    Borrowed {
        // This field refs the segment so its data does not get munmap'd and remains valid.
        _ref: Arc<Segment>,
        // This data point to the ref-counted `_pin` field.
        // Maybe if polonius supports self-referential slices one day, this
        // spicy unsafe code can be dropped.
        ptr: NonNull<u8>,
        len: usize,
    },
    Owned(String),
}

impl SegStr {
    /// Constructs a string that might borrows data from a [Segment]. If the data
    /// is invalid utf-8, it will be converted into an owned [String] using `String::from_utf8_lossy`.
    pub fn from_bytes(bytes: SegBytes) -> Self {
        match bytes.0 {
            SegBytesRepr::Borrowed { _ref, ptr, len } => {
                // Safety: by construction of SegBytes
                let data = unsafe { std::slice::from_raw_parts(ptr.as_ptr(), len) };
                match String::from_utf8_lossy(data) {
                    Cow::Owned(s) => Self(SegStrRepr::Owned(s)),
                    Cow::Borrowed(_) => Self(SegStrRepr::Borrowed { ptr, len, _ref }),
                }
            }
            SegBytesRepr::Owned(b) => match String::from_utf8_lossy(&b) {
                Cow::Owned(s) => Self(SegStrRepr::Owned(s)),
                Cow::Borrowed(_) => {
                    // Safety: We already checked that the data is valid utf-8
                    //         in the `String::from_utf8_lossy` call.
                    Self(SegStrRepr::Owned(unsafe { String::from_utf8_unchecked(b) }))
                }
            },
        }
    }

    /// Returns a byte slice of this [SegStr]'s components.
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        // Safety: We have already checked in the constructor.
        match &self.0 {
            SegStrRepr::Borrowed { ptr, len, .. } => unsafe {
                std::slice::from_raw_parts(ptr.as_ptr(), *len)
            },
            SegStrRepr::Owned(s) => s.as_bytes(),
        }
    }

    /// Extract a [str] slice backed by the pinned segment data or owned data.
    #[inline]
    pub fn as_str(&self) -> &str {
        // Safety: we already did utf-8 checking
        unsafe { std::str::from_utf8_unchecked(self.as_bytes()) }
    }
}

impl std::borrow::Borrow<str> for SegStr {
    #[inline]
    fn borrow(&self) -> &str {
        self
    }
}

impl std::ops::Deref for SegStr {
    type Target = str;

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl std::convert::AsRef<str> for SegStr {
    #[inline]
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl std::fmt::Debug for SegStr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self.as_str(), f)
    }
}
