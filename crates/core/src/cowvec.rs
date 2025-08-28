use arc_swap::ArcSwap;
use std::alloc::{self, Layout};
use std::ops::Deref;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

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
    #[inline]
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
    fn allocate_copy(&self, len: usize, new_cap: Option<usize>) -> Self {
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
pub struct CowVecWriter<T> {
    target: Arc<CowVec<T>>,
}

impl<T> CowVecWriter<T>
where
    T: Copy,
{
    /// Appends an element to the back of this collection.
    ///
    /// This operation is O(1) amortized.
    pub fn push(&mut self, elem: T) {
        let buf = self.target.buf.load();
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

    /// Inserts an element at the given index, shifting all elements after it to the right.
    ///
    /// This operation is O(n) where n is the number of elements to the right of the index.
    /// It will also always perform an allocation before swapping out the internal buffer.
    #[allow(dead_code)]
    pub fn insert(&mut self, index: usize, elem: T) {
        // Unlike push, we can observe the buffer changing underneath us
        // in the case of concurrent readers. So we need to allocate a new
        // buffer every time.

        let buf = self.target.buf.load();
        let len = buf.len.load(Ordering::Acquire);

        assert!(index <= len, "index out of bounds");
        let mut new_buf = if buf.cap == len {
            buf.allocate_copy(index, None)
        } else {
            buf.allocate_copy(index, Some(buf.cap))
        };

        unsafe {
            // Copy second part of old slice into destination
            std::ptr::copy_nonoverlapping(
                buf.as_ptr().add(index),
                new_buf.as_ptr().add(index + 1),
                len - index,
            );
            std::ptr::write(new_buf.as_ptr().add(index), elem);
        }

        *new_buf.len.get_mut() = len + 1;

        self.target.buf.store(Arc::new(new_buf))
    }

    /// Reserves capacity for at least `additional` more elements to be inserted
    /// in the given `Cow Vec<T>`. The collection may reserve more space to
    /// speculatively avoid frequent reallocations. After calling `reserve`,
    /// capacity will be greater than or equal to `self.len() + additional`.
    /// Does nothing if capacity is already sufficient.
    pub fn reserve(&mut self, additional: usize) {
        let buf = self.target.buf.load();
        let len = buf.len.load(Ordering::Acquire);
        if len.saturating_add(additional) > buf.cap {
            self.grow(&buf, len, Some(buf.cap + additional));
        }
    }

    /// Grow will return a buffer that the caller can write to.
    fn grow(&mut self, buf: &RawBuf<T>, len: usize, new_cap: Option<usize>) -> Arc<RawBuf<T>> {
        let ret = Arc::new(buf.allocate_copy(len, new_cap));
        self.target.buf.store(ret.clone());
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
        let buf = self.target.buf.load();
        let len = buf.len.load(Ordering::SeqCst);
        unsafe { std::slice::from_raw_parts(buf.as_ptr(), len) }
    }
}

impl<T> Drop for CowVecWriter<T> {
    fn drop(&mut self) {
        // Mark the CowVec as completed when the writer is dropped
        self.target.completed.store(true, Ordering::Release);
    }
}

/// A contiguous, growable array type, written as `CowVec<T>`.
///
/// Cloning this vector will give another read-handle to the same underlying
/// buffer. This is useful for sharing data between threads.
///
/// This vector has **amortized O(1)** `push()` operation and **O(1)** `clone()`
/// operations. All `CowVec<T>` share the same underlying buffer, so cloning
/// so changes are reflected across all clones.
///
/// The `CowVecWriter<T>` type is an exclusive writer to a `CowVec<T>`.
pub struct CowVec<T> {
    buf: ArcSwap<RawBuf<T>>,
    completed: AtomicBool,
}

impl<T> CowVec<T> {
    /// Constructs a new, empty `CowVec<T>` with a write handle.
    ///
    /// The vector will not allocate until elements are pushed onto it.
    #[inline]
    pub fn new() -> (Arc<Self>, CowVecWriter<T>) {
        assert!(std::mem::size_of::<T>() != 0);
        let buf = ArcSwap::from_pointee(RawBuf::empty());
        let buf = Arc::new(Self {
            buf,
            completed: AtomicBool::new(false),
        });
        (buf.clone(), CowVecWriter { target: buf })
    }

    /// Constructs a new, empty `CowVec<T>` with at least the specified capacity.
    ///
    /// The vector will be able to hold at least `capacity` elements without
    /// reallocating. This method is allowed to allocate for more elements than
    /// `capacity`. If `capacity` is 0, the vector will not allocate.
    #[allow(dead_code)]
    pub fn with_capacity(cap: usize) -> (Arc<Self>, CowVecWriter<T>) {
        assert!(std::mem::size_of::<T>() != 0);
        let buf = ArcSwap::from_pointee(RawBuf::allocate(0, cap));
        let buf = Arc::new(Self {
            buf,
            completed: AtomicBool::new(false),
        });
        (buf.clone(), CowVecWriter { target: buf })
    }

    /// Constructs a new, empty `CowVec<T>`.
    #[inline]
    pub fn empty() -> Self {
        let buf = Self::new().0;
        Arc::try_unwrap(buf).unwrap()
    }

    /// Returns the number of elements in the vector, also referred to as its ‘length’.
    pub fn len(&self) -> usize {
        self.read(|slice| slice.len())
    }

    /// Returns true if the vector contains no elements.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns true if the corresponding CowVecWriter has been dropped.
    ///
    /// When this returns true, no more elements can be added to this vector.
    pub fn is_complete(&self) -> bool {
        self.completed.load(Ordering::Acquire)
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
    #[allow(dead_code)]
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
            buf: ArcSwap::from_pointee(RawBuf::new(NonNull::new(ptr).unwrap(), len, cap)),
            completed: AtomicBool::new(true), // Vec is already complete, no writer exists
        }
    }
}

impl<T> std::fmt::Debug for CowVec<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[..]")
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

    #[test]
    fn test_miri_insert() {
        let (arr, mut writer) = CowVec::new();
        for i in (0..100).step_by(10) {
            writer.push(i);
        }

        let expected = [0, 10, 20, 30, 40, 50, 60, 70, 80, 90];
        for (i, expected) in expected.into_iter().enumerate() {
            assert_eq!(Some(expected), arr.get(i));
        }

        writer.insert(1, 5);
        let expected = [0, 5, 10, 20, 30, 40, 50, 60, 70, 80, 90];
        for (i, expected) in expected.into_iter().enumerate() {
            assert_eq!(Some(expected), arr.get(i));
        }

        writer.insert(1, 5);
        let expected = [0, 5, 5, 10, 20, 30, 40, 50, 60, 70, 80, 90];
        for (i, expected) in expected.into_iter().enumerate() {
            assert_eq!(Some(expected), arr.get(i));
        }
    }

    #[test]
    fn test_completed_flag() {
        let (arr, writer) = CowVec::<i32>::new();

        // Initially, the vector should not be completed
        assert!(!arr.is_complete());

        // Drop the writer
        drop(writer);

        // Now the vector should be completed
        assert!(arr.is_complete());
    }

    #[test]
    fn test_completed_flag_from_vec() {
        let vec = vec![1, 2, 3, 4, 5];
        let arr = CowVec::from(vec);

        // CowVec created from Vec should be immediately completed
        assert!(arr.is_complete());
    }
}
