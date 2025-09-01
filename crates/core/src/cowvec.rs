use std::{
    alloc::{self, Layout},
    ops::Deref,
    ptr::NonNull,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
};

use arc_swap::ArcSwap;

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
        let len = buf.len.load(Ordering::Relaxed);
        let cap = buf.cap;

        let push_inner = move |buf: &RawBuf<T>, len, elem| {
            unsafe { std::ptr::write(buf.ptr.as_ptr().add(len), elem) }
            buf.len.store(len + 1, Ordering::Release);
        };

        if len == cap {
            // Safety: If this runs, then buf will no longer be borrowed from
            push_inner(&self.grow(&buf, len, None), len, elem)
        } else {
            push_inner(&buf, len, elem)
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
        let len = buf.len.load(Ordering::Relaxed);

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
        let len = buf.len.load(Ordering::Relaxed);
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

    pub fn has_readers(&self) -> bool {
        Arc::strong_count(&self.target) > 1
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
        let len = buf.len.load(Ordering::Acquire);
        unsafe { std::slice::from_raw_parts(buf.as_ptr(), len) }
    }
}

impl<T> Drop for CowVecWriter<T> {
    fn drop(&mut self) {
        self.target.completed.store(true, Ordering::Relaxed);
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
        self.completed.load(Ordering::Relaxed)
    }

    #[inline(always)]
    fn read<F, R>(&self, cb: F) -> R
    where
        F: FnOnce(&[T]) -> R,
    {
        let buf = self.buf.load();
        let len = buf.len.load(Ordering::Acquire);
        cb(unsafe { std::slice::from_raw_parts(buf.as_ptr(), len) })
    }

    /// Returns a snapshot of the current state of the vector.
    ///
    /// This refs/pins the current internal buffer. Users can read
    /// up to `len()` elements at the time of the snapshot.
    pub fn snapshot(&self) -> CowVecSnapshot<T> {
        let buf = self.buf.load_full();
        CowVecSnapshot {
            len: buf.len.load(Ordering::Acquire),
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
    use std::sync::Arc;
    use std::time::Duration;

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

    #[test]
    fn test_completion_with_concurrent_readers() {
        use std::sync::Barrier;
        use std::thread;

        let (arr, mut writer) = CowVec::<i32>::new();
        let barrier = Arc::new(Barrier::new(2));

        // Clone the vector for reader
        let arr_clone = arr.clone();
        let barrier_clone = barrier.clone();

        // Reader thread
        let handle = thread::spawn(move || {
            barrier_clone.wait();

            // Keep checking until we see completion
            let mut saw_incomplete = false;
            let mut saw_complete = false;

            for _ in 0..1000 {
                let is_complete = arr_clone.is_complete();
                if !is_complete {
                    saw_incomplete = true;
                } else {
                    saw_complete = true;
                    break;
                }
                thread::sleep(Duration::from_micros(100));
            }

            (saw_incomplete, saw_complete)
        });

        // Writer thread (main)
        barrier.wait();

        // Add some elements
        for i in 0..5 {
            writer.push(i);
            thread::sleep(Duration::from_millis(1));
        }

        // Ensure reader has time to see incomplete state
        thread::sleep(Duration::from_millis(10));

        // Drop writer
        drop(writer);

        let (saw_incomplete, saw_complete) = handle.join().unwrap();

        // Reader should have seen both states
        assert!(saw_incomplete, "Reader should have seen incomplete state");
        assert!(saw_complete, "Reader should have seen complete state");
        assert!(arr.is_complete(), "Final state should be complete");
    }

    #[test]
    fn test_edge_case_empty_vector_completion() {
        let (arr, writer) = CowVec::<u8>::new();

        // Empty vector should not be complete while writer exists
        assert!(!arr.is_complete());
        assert_eq!(arr.len(), 0);
        assert!(arr.is_empty());

        drop(writer);

        // Empty vector should be complete after writer is dropped
        assert!(arr.is_complete());
        assert_eq!(arr.len(), 0);
        assert!(arr.is_empty());
    }

    #[test]
    fn test_edge_case_single_element() {
        let (arr, mut writer) = CowVec::new();

        assert!(!arr.is_complete());
        writer.push(42);
        assert_eq!(arr.len(), 1);
        assert_eq!(arr.get(0), Some(42));
        assert!(!arr.is_complete());

        drop(writer);

        assert!(arr.is_complete());
        assert_eq!(arr.len(), 1);
        assert_eq!(arr.get(0), Some(42));
    }

    #[test]
    fn test_completion_with_capacity_growth() {
        let (arr, mut writer) = CowVec::with_capacity(2);

        assert!(!arr.is_complete());

        // Fill initial capacity
        writer.push(1);
        writer.push(2);
        assert!(!arr.is_complete());

        // Force reallocation
        writer.push(3);
        writer.push(4);
        assert!(!arr.is_complete());

        drop(writer);
        assert!(arr.is_complete());
        assert_eq!(arr.len(), 4);
    }

    #[test]
    fn test_completion_status_across_clones() {
        let (arr, writer) = CowVec::<i32>::new();
        let clone1 = arr.clone();
        let clone2 = arr.clone();

        // All clones should show not complete
        assert!(!arr.is_complete());
        assert!(!clone1.is_complete());
        assert!(!clone2.is_complete());

        drop(writer);

        // All clones should show complete
        assert!(arr.is_complete());
        assert!(clone1.is_complete());
        assert!(clone2.is_complete());
    }

    #[test]
    fn test_completion_with_insert_operations() {
        let (arr, mut writer) = CowVec::new();

        // Add some initial elements
        for i in 0..5 {
            writer.push(i * 10);
        }
        assert!(!arr.is_complete());

        // Perform insert operations
        writer.insert(2, 15);
        writer.insert(0, -5);
        assert!(!arr.is_complete());

        let expected = [-5, 0, 10, 15, 20, 30, 40];
        for (i, &expected) in expected.iter().enumerate() {
            assert_eq!(arr.get(i), Some(expected));
        }

        drop(writer);
        assert!(arr.is_complete());

        // Data should still be accessible after completion
        for (i, &expected) in expected.iter().enumerate() {
            assert_eq!(arr.get(i), Some(expected));
        }
    }

    #[test]
    fn test_completion_stress_test() {
        use std::thread;

        // Test with many concurrent readers checking completion status
        let (arr, mut writer) = CowVec::<usize>::new();
        let mut handles = Vec::new();

        // Spawn multiple reader threads
        for thread_id in 0..10 {
            let arr_clone = arr.clone();
            let handle = thread::spawn(move || {
                let mut completion_changes = 0;
                let mut last_complete = arr_clone.is_complete();

                for _ in 0..100 {
                    let current_complete = arr_clone.is_complete();
                    if current_complete != last_complete {
                        completion_changes += 1;
                        last_complete = current_complete;
                    }
                    thread::sleep(Duration::from_micros(100));
                }

                (thread_id, completion_changes, last_complete)
            });
            handles.push(handle);
        }

        // Writer adds elements then gets dropped
        for i in 0..50 {
            writer.push(i);
            if i % 10 == 0 {
                thread::sleep(Duration::from_millis(1));
            }
        }

        thread::sleep(Duration::from_millis(5));
        drop(writer);

        // Wait for all readers and verify results
        for handle in handles {
            let (thread_id, changes, final_complete) = handle.join().unwrap();
            assert!(final_complete, "Thread {} should see final completion", thread_id);
            // Each thread should see at most one change (false -> true)
            assert!(changes <= 1, "Thread {} saw {} completion changes", thread_id, changes);
        }

        assert!(arr.is_complete());
        assert_eq!(arr.len(), 50);
    }

    #[test]
    fn test_completion_with_small_types() {
        let (arr, mut writer) = CowVec::<u8>::new();

        assert!(!arr.is_complete());

        // Push some small values
        for i in 0..10 {
            writer.push(i);
        }

        assert_eq!(arr.len(), 10);
        assert!(!arr.is_complete());

        drop(writer);

        assert!(arr.is_complete());
        assert_eq!(arr.len(), 10);
    }

    #[test]
    fn test_completion_with_large_elements() {
        #[derive(Copy, Clone, PartialEq, Debug)]
        struct LargeStruct {
            data: [u64; 16], // 128 bytes
        }

        let (arr, mut writer) = CowVec::<LargeStruct>::new();
        let large_elem = LargeStruct { data: [42; 16] };

        assert!(!arr.is_complete());

        writer.push(large_elem);
        assert_eq!(arr.len(), 1);
        assert_eq!(arr.get(0), Some(large_elem));
        assert!(!arr.is_complete());

        drop(writer);

        assert!(arr.is_complete());
        assert_eq!(arr.get(0), Some(large_elem));
    }

    #[test]
    fn test_completion_snapshot_consistency() {
        let (arr, mut writer) = CowVec::new();

        // Add some data
        for i in 0..10 {
            writer.push(i);
        }

        // Take snapshot before completion
        let snapshot_before = arr.snapshot();
        assert!(!arr.is_complete());

        drop(writer);

        // Take snapshot after completion
        let snapshot_after = arr.snapshot();
        assert!(arr.is_complete());

        // Snapshots should have same data
        assert_eq!(snapshot_before.len(), snapshot_after.len());
        for i in 0..snapshot_before.len() {
            assert_eq!(snapshot_before[i], snapshot_after[i]);
        }
    }

    #[test]
    fn test_completion_with_reserve_operations() {
        let (arr, mut writer) = CowVec::new();

        assert!(!arr.is_complete());

        // Reserve capacity
        writer.reserve(100);
        assert!(!arr.is_complete());

        // Add some elements
        for i in 0..10 {
            writer.push(i);
        }
        assert!(!arr.is_complete());

        drop(writer);

        assert!(arr.is_complete());
        assert_eq!(arr.len(), 10);
    }

    #[test]
    fn test_completion_rapid_drop_and_check() {
        // Test rapid succession of drop and completion check
        for _ in 0..100 {
            let (arr, writer) = CowVec::<i32>::new();
            assert!(!arr.is_complete());
            drop(writer);
            assert!(arr.is_complete());
        }
    }

    #[test]
    fn test_atomic_ordering_optimization() {
        use std::thread;

        // Test that relaxed ordering works correctly under high contention
        let (arr, mut writer) = CowVec::<usize>::new();
        let mut handles = Vec::new();

        // Spawn many threads that rapidly check completion status
        for thread_id in 0..20 {
            let arr_clone = arr.clone();
            let handle = thread::spawn(move || {
                let mut check_count = 0;
                let start = std::time::Instant::now();

                // Rapidly check completion for 10ms
                while start.elapsed() < Duration::from_millis(10) {
                    let _ = arr_clone.is_complete();
                    check_count += 1;
                }

                // Final check after writer should be dropped
                thread::sleep(Duration::from_millis(5));
                let final_complete = arr_clone.is_complete();

                (thread_id, check_count, final_complete)
            });
            handles.push(handle);
        }

        // Writer does some work then gets dropped
        for i in 0..10 {
            writer.push(i);
        }

        // Small delay then drop
        thread::sleep(Duration::from_millis(2));
        drop(writer);

        // Verify all threads eventually see completion
        for handle in handles {
            let (thread_id, check_count, final_complete) = handle.join().unwrap();
            assert!(final_complete, "Thread {} should see completion", thread_id);
            assert!(check_count > 0, "Thread {} should have performed checks", thread_id);
        }

        assert!(arr.is_complete());
    }

    #[test]
    fn test_memory_ordering_under_contention() {
        use std::thread;

        // Test that optimized memory ordering works correctly under high contention
        let (arr, mut writer) = CowVec::<usize>::new();
        let mut handles = Vec::new();

        // Spawn many reader threads that access data while writer is active
        for thread_id in 0..10 {
            let arr_clone = arr.clone();
            let handle = thread::spawn(move || {
                let mut successful_reads = 0;
                let start = std::time::Instant::now();

                // Continuously read data for 20ms
                while start.elapsed() < Duration::from_millis(20) {
                    let len = arr_clone.len();
                    // Verify we can read all elements up to the observed length
                    for i in 0..len {
                        if let Some(value) = arr_clone.get(i) {
                            // Verify data integrity: each element should equal its index
                            assert_eq!(value, i, "Thread {} saw corrupted data at index {}", thread_id, i);
                            successful_reads += 1;
                        }
                    }
                }

                (thread_id, successful_reads)
            });
            handles.push(handle);
        }

        // Writer rapidly adds elements
        for i in 0..100 {
            writer.push(i);
            // Occasional yield to increase contention
            if i % 20 == 0 {
                thread::yield_now();
            }
        }

        drop(writer);

        // Verify all readers completed successfully
        let mut total_reads = 0;
        for handle in handles {
            let (thread_id, reads) = handle.join().unwrap();
            assert!(reads > 0, "Thread {} should have performed successful reads", thread_id);
            total_reads += reads;
        }

        assert!(total_reads > 0);
        assert!(arr.is_complete());
        assert_eq!(arr.len(), 100);

        // Final verification of data integrity
        for i in 0..100 {
            assert_eq!(arr.get(i), Some(i));
        }
    }
}
