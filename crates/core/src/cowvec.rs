use arc_swap::ArcSwap;
use std::{
    alloc::{self, Layout},
    marker::PhantomData,
    ops::Deref,
    ptr::NonNull,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

struct RawBuf<T> {
    ptr: NonNull<T>,
    len: AtomicUsize,
    cap: usize,
}

impl<T> RawBuf<T> {
    #[inline]
    const fn new(ptr: NonNull<T>, len: usize, cap: usize) -> Self {
        Self {
            ptr,
            len: AtomicUsize::new(len),
            cap,
        }
    }

    #[inline]
    const fn empty() -> Self {
        Self::new(std::ptr::NonNull::dangling(), 0, 0)
    }

    /// Allocate a new buffer with the given capacity.
    fn allocate(init_len: usize, cap: usize) -> Self {
        if cap == 0 {
            return Self::empty();
        }

        // `Layout::array` checks that the number of bytes is <= usize::MAX,
        // but this is redundant since old_layout.size() <= isize::MAX,
        // so the `unwrap` should never fail.
        let layout = Layout::array::<T>(cap).unwrap();

        // Ensure that the new allocation doesn't exceed `isize::MAX` bytes.
        assert!(layout.size() <= isize::MAX as usize, "Allocation too large");

        let ptr = unsafe { alloc::alloc(layout) };

        // If allocation fails, `new_ptr` will be null, in which case we abort.
        let Some(new_ptr) = NonNull::new(ptr.cast::<T>()) else {
            alloc::handle_alloc_error(layout)
        };

        RawBuf::new(new_ptr, init_len, cap)
    }
}

impl<T> RawBuf<T>
where
    T: Copy,
{
    /// Return a new buffer with the same contents, but with a larger capacity.
    fn grow(&self, len: usize, new_cap: Option<usize>) -> Self {
        let new_cap = new_cap.unwrap_or((self.cap * 2).max(1));
        debug_assert!(new_cap >= self.cap);

        let new_buf = Self::allocate(len, new_cap);
        if self.cap != 0 {
            let old_ptr = self.ptr.as_ptr().cast::<u8>();
            // Cannot use realloc here since it may drop the old pointer
            let new_ptr = new_buf.ptr.as_ptr().cast::<u8>();
            // This is fine since our elements are Copy
            let old_layout_len = Layout::array::<T>(len).unwrap();
            unsafe { std::ptr::copy_nonoverlapping(old_ptr, new_ptr, old_layout_len.size()) };
        }
        new_buf
    }
}

impl<T> Deref for RawBuf<T> {
    type Target = NonNull<T>;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.ptr
    }
}

unsafe impl<T: Send> Send for RawBuf<T> {}
unsafe impl<T: Sync> Sync for RawBuf<T> {}

impl<T> Drop for RawBuf<T> {
    fn drop(&mut self) {
        let cap = self.cap;
        if cap != 0 {
            // Safety: we are the last owner, we can do a relaxed read of len
            unsafe {
                std::ptr::drop_in_place(std::ptr::slice_from_raw_parts_mut(
                    self.ptr.as_ptr(),
                    self.len.load(Ordering::Relaxed),
                ));
            }
            unsafe {
                alloc::dealloc(
                    self.ptr.as_ptr().cast::<u8>(),
                    Layout::array::<T>(cap).unwrap(),
                );
            }
        }
    }
}

/// An exclusive writer to a `CowVec<T>`.
///
/// This is useful for pushing elements to the back of the vector.
pub struct CowVecWriter<T> {
    buf: Arc<ArcSwap<RawBuf<T>>>,
}

impl<T> CowVecWriter<T>
where
    T: Copy,
{
    pub fn has_readers(&self) -> bool {
        Arc::strong_count(&self.buf) > 1
    }

    /// Appends an element to the back of this collection.
    pub fn push(&mut self, elem: T) {
        let buf = self.buf.load();
        let len = buf.len.load(Ordering::Acquire);
        let cap = buf.cap;

        let push_inner = move |buf: &RawBuf<T>| {
            unsafe { std::ptr::write(buf.ptr.as_ptr().add(len), elem) }
            buf.len.store(len + 1, Ordering::Release);
        };

        if len == cap {
            // Safety: If this runs, then buf will no longer be borrowed from
            push_inner(&self.grow(&buf, len, None))
        } else {
            push_inner(&buf)
        }
    }

    pub fn reserve(&mut self, new_cap: usize) {
        let buf = self.buf.load();

        if new_cap > buf.cap {
            let len = buf.len.load(Ordering::Acquire);
            self.grow(&buf, len, Some(new_cap));
        }
    }

    /// Grow will return a buffer that the caller can write to.
    fn grow(&mut self, buf: &RawBuf<T>, len: usize, new_cap: Option<usize>) -> Arc<RawBuf<T>> {
        let ret = Arc::new(buf.grow(len, new_cap));
        self.buf.store(ret.clone());
        ret
    }
}

impl<T> Deref for CowVecWriter<T> {
    type Target = [T];

    #[inline]
    fn deref(&self) -> &Self::Target {
        // Safety: the writer itself pins the buffer, so it is safe to read
        //         from it as long as the lifetime prevents the writer from
        //         growing reallocating the internal buffer.
        let buf = self.buf.load();
        let len = buf.len.load(Ordering::SeqCst);
        unsafe { std::slice::from_raw_parts(buf.as_ptr(), len) }
    }
}

/// A contiguous, growable, append-only array type, written as `CowVec<T>`.
///
/// Cloning this vector will give another read-handle to the same underlying
/// buffer. This is useful for sharing data between threads.
///
/// This vector has **amortized O(1)** `push()` operation and **O(1)** `clone()`
/// operations.
#[derive(Clone)]
pub struct CowVec<T> {
    buf: Arc<ArcSwap<RawBuf<T>>>,
}

impl<T> CowVec<T> {
    /// Constructs a new, empty `CowVec<T>` with a write handle.
    ///
    /// The vector will not allocate until elements are pushed onto it.
    #[inline]
    pub fn new() -> (Self, CowVecWriter<T>) {
        assert!(std::mem::size_of::<T>() != 0);
        let buf = Arc::new(ArcSwap::from_pointee(RawBuf::empty()));
        (Self { buf: buf.clone() }, CowVecWriter { buf })
    }

    pub fn with_capacity(cap: usize) -> (Self, CowVecWriter<T>) {
        assert!(std::mem::size_of::<T>() != 0);
        let buf = Arc::new(ArcSwap::from_pointee(RawBuf::allocate(0, cap)));
        (Self { buf: buf.clone() }, CowVecWriter { buf })
    }

    /// Constructs a new, empty `CowVec<T>`.
    pub fn empty() -> Self {
        Self::new().0
    }

    /// Returns the number of elements in the vector, also referred to as its ‘length’.
    pub fn len(&self) -> usize {
        self.read(|slice| slice.len())
    }

    /// Returns true if the vector contains no elements.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline(always)]
    fn read<F, R>(&self, cb: F) -> R
    where
        F: FnOnce(&[T]) -> R,
    {
        let buf = self.buf.load();
        let len = buf.len.load(Ordering::SeqCst);
        cb(unsafe { std::slice::from_raw_parts(buf.as_ptr(), len) })
    }

    /// Returns a snapshot of the current state of the vector.
    ///
    /// This refs/pins the current internal buffer. Users can read
    /// up to `len()` elements at the time of the snapshot.
    pub fn snapshot(&self) -> CowVecSnapshot<T> {
        let buf = self.buf.load_full();
        CowVecSnapshot {
            len: buf.len.load(Ordering::SeqCst),
            buf,
        }
    }
}

impl<T> CowVec<T>
where
    T: Copy,
{
    /// Returns the element at the given index, or `None` if out of bounds.
    pub fn get(&self, index: usize) -> Option<T> {
        self.read(|slice| slice.get(index).copied())
    }

    /// Returns the element at the given index.
    pub unsafe fn get_unchecked(&self, index: usize) -> T {
        self.get(index).unwrap_unchecked()
    }
}

#[macro_export]
macro_rules! cowvec {
    () => (
        $crate::vec::CowVec::new()
    );
    ($($x:expr),+ $(,)?) => ({
        let mut vec = $crate::cowvec::CowVec::new();
        $(vec.push($x);)+
        vec
    });
}

impl<T: Copy> From<Vec<T>> for CowVec<T> {
    fn from(vec: Vec<T>) -> Self {
        let mut me = std::mem::ManuallyDrop::new(vec);
        let (ptr, len, cap) = (me.as_mut_ptr(), me.len(), me.capacity());

        Self {
            buf: Arc::new(ArcSwap::from_pointee(RawBuf::new(
                NonNull::new(ptr).unwrap(),
                len,
                cap,
            ))),
        }
    }
}

pub struct CowVecSnapshot<T> {
    buf: Arc<RawBuf<T>>,
    len: usize,
}

impl<T> CowVecSnapshot<T>
where
    T: Copy,
{
    /// Returns the element at the given index, or `None` if out of bounds.
    pub fn get(&self, index: usize) -> Option<T> {
        self.deref().get(index).copied()
    }

    /// Returns the element at the given index.
    pub unsafe fn get_unchecked(&self, index: usize) -> T {
        self.get(index).unwrap_unchecked()
    }

    /// Extracts a slice containing the entire vector.
    ///
    /// Equivalent to `&s[..]`.
    pub fn as_slice(&self) -> &[T] {
        self
    }
}

impl<T> Deref for CowVecSnapshot<T> {
    type Target = [T];

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        // Safety: the snapshot pins the buffer, so it is safe to read from it
        let buf = &self.buf;
        let len = self.len;
        unsafe { std::slice::from_raw_parts(buf.as_ptr(), len) }
    }
}

#[cfg(test)]
mod test {
    use super::CowVec;

    #[test]
    fn test_miri_push_and_access() {
        let (arr, mut writer) = CowVec::new();
        for i in 0..10000 {
            writer.push(i);
        }
        for i in 0..10000 {
            assert_eq!(Some(i), arr.get(i));
        }
    }

    #[test]
    fn test_miri_push_and_concurrent_access() {
        let (arr, mut writer) = CowVec::new();
        let handle = std::thread::spawn({
            move || {
                for _ in 0..10 {
                    for i in 0..1000 {
                        writer.push(i);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
            }
        });

        while !handle.is_finished() {
            for i in 0..arr.len() {
                assert_eq!(Some(i % 1000), arr.get(i));
            }
        }

        handle.join().unwrap();
    }

    #[test]
    fn test_miri_push_and_concurrent_access_snapshot() {
        let (arr, mut writer) = CowVec::new();
        let handle = std::thread::spawn({
            move || {
                for _ in 0..10 {
                    for i in 0..1000 {
                        writer.push(i);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
            }
        });

        while !handle.is_finished() {
            let slice = arr.snapshot();
            for i in slice.iter().copied() {
                assert_eq!(i, slice[i]);
            }
        }

        handle.join().unwrap();
    }

    #[test]
    fn test_miri_clone() {
        let (arr, mut writer) = CowVec::new();
        for i in 0..10 {
            writer.push(i);
        }
        let cloned_arr = arr.clone();
        assert_eq!(arr.len(), cloned_arr.len());
        for i in 0..10 {
            assert_eq!(arr.get(i), cloned_arr.get(i));
        }
        writer.push(10);
        assert_eq!(arr.get(10), cloned_arr.get(10));
        assert_eq!(arr.len(), cloned_arr.len());
    }

    #[test]
    fn test_miri_deref() {
        let (arr, mut writer) = CowVec::new();
        for i in 0..10 {
            writer.push(i);
        }
        let snap = arr.snapshot();
        let slice: &[i32] = &snap;
        assert_eq!(slice.len(), arr.len());
        for i in 0..10 {
            assert_eq!(slice.get(i).copied(), arr.get(i));
            assert_eq!(snap.get(i), arr.get(i));
        }
    }

    #[test]
    fn test_miri_with_capacity() {
        let (arr, mut writer) = CowVec::with_capacity(100);
        let init_ptr = arr.buf.load().as_ptr();
        for i in 0..100 {
            writer.push(i);
        }
        let mid_ptr = arr.buf.load().as_ptr();
        assert_eq!(init_ptr, mid_ptr);
        writer.push(100);
        let final_ptr = arr.buf.load().as_ptr();
        assert_ne!(mid_ptr, final_ptr);
    }

    
    #[test]
    fn test_miri_reserve() {
        let (arr, mut writer) = CowVec::new();
        writer.reserve(100);
        let init_ptr = arr.buf.load().as_ptr();
        for i in 0..100 {
            writer.push(i);
        }
        let mid_ptr = arr.buf.load().as_ptr();
        assert_eq!(init_ptr, mid_ptr);
        writer.push(100);
        let final_ptr = arr.buf.load().as_ptr();
        assert_ne!(mid_ptr, final_ptr);
    }
}
