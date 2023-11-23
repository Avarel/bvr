use crate::Result;
use memmap2::{Mmap, MmapMut};
use std::{borrow::Cow, ops::Range, ptr::NonNull, sync::Arc};

#[cfg(unix)]
pub(crate) use std::os::fd::AsRawFd as Mmappable;
#[cfg(windows)]
pub(crate) use std::os::windows::io::AsRawHandle as Mmappable;

pub struct SegmentRaw<Buf> {
    id: usize,
    range: Range<u64>,
    data: Buf,
}

pub type SegmentMut = SegmentRaw<MmapMut>;
pub type Segment = SegmentRaw<Mmap>;

impl<Buf> SegmentRaw<Buf>
where
    Buf: AsRef<[u8]>,
{
    pub const MAX_SIZE: u64 = 1 << 20;

    pub fn id(&self) -> usize {
        self.id
    }

    pub fn start(&self) -> u64 {
        self.range.start
    }

    pub fn translate_inner_data_index(&self, start: u64) -> u64 {
        debug_assert!(self.range.start <= start);
        // TODO: make this better... i don't like that its <=
        //       but technically its fine as long as start
        //       is the end of the buffer
        debug_assert!(start <= self.range.end);
        start - self.range.start
    }

    pub fn translate_inner_data_range(&self, start: u64, end: u64) -> Range<u64> {
        self.translate_inner_data_index(start)..self.translate_inner_data_index(end)
    }

    pub fn id_of_data(start: u64) -> usize {
        (start / Self::MAX_SIZE) as usize
    }

    pub fn data_range_of_id(id: usize) -> Range<u64> {
        let start = id as u64 * Self::MAX_SIZE;
        start..start + Self::MAX_SIZE
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
    pub(crate) fn new(id: usize, start: u64) -> Result<Self> {
        let data = memmap2::MmapOptions::new()
            .len(Self::MAX_SIZE as usize)
            .map_anon()?;
        #[cfg(unix)]
        data.advise(memmap2::Advice::Sequential)?;
        Ok(Self { id, data, range: start..start + Self::MAX_SIZE })
    }

    pub fn into_read_only(self) -> Result<Segment> {
        Ok(Segment {
            id: self.id,
            data: self.data.make_read_only()?,
            range: self.range,
        })
    }
}

impl Segment {
    pub(crate) fn map_file<F: Mmappable>(id: usize, range: Range<u64>, file: &F) -> Result<Self> {
        let size = range.end - range.start;
        debug_assert!(size <= Self::MAX_SIZE);
        let data = unsafe {
            memmap2::MmapOptions::new()
                .offset(range.start)
                .len(size as usize)
                .map(file)?
        };
        #[cfg(unix)]
        data.advise(memmap2::Advice::WillNeed)?;
        Ok(Self {
            id,
            data,
            range
        })
    }

    pub fn get_line(self: &Arc<Self>, range: Range<u64>) -> SegStr {
        let data = &self.data[range.start as usize..range.end as usize];
        // Safety: The length is computed by a (assumed to be correct)
        //         index. It is undefined behavior if the file changes
        //         in a non-appending way after the index is created.
        SegStr::new(self.clone(), data)
    }
}

/// Line string that comes from a [Segment].
///
/// If the [SegStr] borrows from the segment, the segment will not be dropped until
/// all of its referents is dropped.
///
/// This structure avoids cloning unnecessarily.
pub struct SegStr(SegStrRepr);

/// Internal representation of [SegStr].
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
    ///
    /// # Safety
    ///
    /// 1. The provided slice must point to data that lives inside the ref-counted [Segment].
    /// 2. The length must encompass a valid range of data inside the [Segment].
    fn new(origin: Arc<Segment>, data: &[u8]) -> Self {
        // Safety: This ptr came from a slice that we prevent from
        //         being dropped by having it inside a ref counter
        match String::from_utf8_lossy(data) {
            Cow::Borrowed(_) => Self(SegStrRepr::Borrowed {
                _ref: origin,
                ptr: unsafe { NonNull::new(data.as_ptr() as *mut _).unwrap_unchecked() },
                len: data.len(),
            }),
            Cow::Owned(s) => Self::new_owned(s),
        }
    }

    /// Constructs a string that owns its data.
    pub fn new_owned(s: String) -> Self {
        Self(SegStrRepr::Owned(s))
    }

    /// Returns a byte slice of this [SegStr]'s components.
    pub fn as_bytes(&self) -> &[u8] {
        // Safety: We have already checked in the constructor.
        match &self.0 {
            SegStrRepr::Borrowed {
                _ref: _pin,
                ptr,
                len,
            } => unsafe { std::slice::from_raw_parts(ptr.as_ptr(), *len) },
            SegStrRepr::Owned(s) => s.as_bytes(),
        }
    }

    /// Extract a [str] slice backed by the pinned segment data or owned data.
    pub fn as_str(&self) -> &str {
        // Safety: we already did utf-8 checking
        unsafe { std::str::from_utf8_unchecked(self.as_bytes()) }
    }
}

impl std::borrow::Borrow<str> for SegStr {
    fn borrow(&self) -> &str {
        self
    }
}

impl std::ops::Deref for SegStr {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl std::convert::AsRef<str> for SegStr {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl std::fmt::Display for SegStr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self)
    }
}

impl std::fmt::Debug for SegStr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&**self, f)
    }
}
