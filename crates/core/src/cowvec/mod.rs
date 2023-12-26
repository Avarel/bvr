//! Contains the [`CowVec`], which is an append-only vector for [Copy]-elements
//! based on the standard library's [Vec].

pub mod inflight;

use std::{
    alloc::{self, Layout},
    fmt::Debug,
    hint::unreachable_unchecked,
    ops::Deref,
    ptr::NonNull,
    sync::{
        atomic::{
            AtomicUsize,
            Ordering::{Acquire, Relaxed, Release, SeqCst},
        },
        Arc,
    },
};

/// An allocation used in a [`CowVec`].
struct RawBuf<T> {
    ptr: NonNull<T>,
    len: AtomicUsize,
    cap: usize,
}

impl<T> RawBuf<T> {
    #[inline]
    const fn empty() -> Self {
        Self::new(std::ptr::NonNull::dangling(), 0, 0)
    }

    #[inline]
    const fn new(ptr: NonNull<T>, len: usize, cap: usize) -> Self {
        Self {
            ptr,
            len: AtomicUsize::new(len),
            cap,
        }
    }
}

impl<T> Deref for RawBuf<T> {
    type Target = NonNull<T>;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.ptr
    }
}

impl<T> Drop for RawBuf<T> {
    fn drop(&mut self) {
        let cap = self.cap;
        if cap != 0 {
            // Safety: we are the last owner, we can do a relaxed read of len
            unsafe {
                std::ptr::drop_in_place(std::ptr::slice_from_raw_parts_mut(
                    self.ptr.as_ptr(),
                    self.len.load(Relaxed),
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

unsafe impl<T: Send> Send for RawBuf<T> {}
unsafe impl<T: Sync> Sync for RawBuf<T> {}

enum CowVecRepr<T> {
    /// Snapshot form of the [`CowVec`]. Reads must be done using
    /// the saved length field.
    Snapshot { buf: Arc<RawBuf<T>>, len: usize },
    /// Owned form of the [`CowVec`]. Reads must be done using the
    /// atomic length.
    Owned { buf: Arc<RawBuf<T>> },
}

/// A contiguous, growable, append-only array type, written as `CowVec<T>`,
/// short for copy-on-write vector.
///
/// Cloning this vector will give a snapshot of the vector's content at the time
/// of clone. The snapshot shares the buffer with the original owning [`CowVec`]
/// until it reallocates or until the user attempts to mutably alter the data.
///
/// This vector has **amortized O(1)** `push()` operation and **O(1)** `clone()`
/// operations.
pub struct CowVec<T> {
    #[doc(hidden)]
    repr: CowVecRepr<T>,
}

impl<T> Debug for CowVec<T> {
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(fmt, "[..., {}]", self.len())
    }
}

impl<T> Default for CowVec<T> {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl<T> CowVec<T> {
    /// Constructs a new, empty `CowVec<T>`.
    ///
    /// The vector will not allocate until elements are pushed onto it.
    #[inline]
    pub fn new() -> Self {
        assert!(std::mem::size_of::<T>() != 0);
        Self {
            repr: CowVecRepr::Owned {
                buf: Arc::new(RawBuf::empty()),
            },
        }
    }

    /// Returns true if the vector contains no elements.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the number of elements in the vector, also referred to as its ‘length’.
    #[inline]
    pub fn len(&self) -> usize {
        // No matter what len we load, it will be valid since the length
        // is only incremented after the data is written.
        match &self.repr {
            CowVecRepr::Snapshot { len, .. } => *len,
            CowVecRepr::Owned { buf } => buf.len.load(Relaxed),
        }
    }

    /// Extracts a slice containing the entire vector.
    ///
    /// Equivalent to `&s[..]`.
    #[inline]
    pub fn as_slice(&self) -> &[T] {
        self
    }
}

impl<T: Copy> From<Vec<T>> for CowVec<T> {
    fn from(vec: Vec<T>) -> Self {
        let mut me = std::mem::ManuallyDrop::new(vec);
        let (ptr, len, cap) = (me.as_mut_ptr(), me.len(), me.capacity());

        Self {
            repr: CowVecRepr::Owned {
                buf: Arc::new(RawBuf::new(NonNull::new(ptr).unwrap(), len, cap)),
            },
        }
    }
}

impl<T: Copy> CowVec<T> {
    fn fixup_mut_buf(&mut self) -> (&Arc<RawBuf<T>>, usize) {
        match self.repr {
            CowVecRepr::Snapshot { len, .. } => (self.grow(true), len),
            CowVecRepr::Owned { ref buf } => {
                let len = buf.len.load(Acquire);
                let cap = buf.cap;
                (
                    if len == cap {
                        // Safety: If this runs, then buf will no longer be borrowed from
                        self.grow(false)
                    } else {
                        // Safety: telling the compiler to shut it because this is valid
                        //         we can return the lifetime of this just fine
                        //         If you're not convinced, just inline this and drop the transmute
                        unsafe { std::mem::transmute(buf) }
                    },
                    len,
                )
            }
        }
    }

    /// Returns the element at the given index, or `None` if out of bounds.
    pub fn get(&self, index: usize) -> Option<T> {
        self.as_slice().get(index).copied()
    }

    /// Appends an element to the back of this collection.
    ///
    /// If the collection is in a borrowed state, it will copy the data
    /// underlying the borrowed state and transition to an owned state.
    pub fn push(&mut self, elem: T) {
        let (buf, len) = self.fixup_mut_buf();

        unsafe { std::ptr::write_volatile(buf.ptr.as_ptr().add(len), elem) }

        // Can't fail, we'll OOM first.
        // There should be no other writers, but lets be safe.
        buf.len.store(len + 1, Release);
    }

    /// Grow will return a buffer that the caller can write to.
    fn grow(&mut self, from_snapshot: bool) -> &Arc<RawBuf<T>> {
        let (buf, len) = match &self.repr {
            CowVecRepr::Snapshot { buf, len } => (buf, *len),
            CowVecRepr::Owned { buf } => (buf, buf.len.load(SeqCst)),
        };
        let cap = buf.cap;

        // since we set the capacity to usize::MAX when T has size 0,
        // getting to here necessarily means the Vec is overfull.
        assert!(std::mem::size_of::<T>() != 0, "capacity overflow");

        let (new_cap, new_layout) = if cap == 0 {
            (1, Layout::array::<T>(1).unwrap())
        } else {
            // This can't overflow because we ensure self.cap <= isize::MAX.
            let new_cap = if from_snapshot {
                (cap + 1).next_power_of_two()
            } else {
                2 * cap
            };

            // `Layout::array` checks that the number of bytes is <= usize::MAX,
            // but this is redundant since old_layout.size() <= isize::MAX,
            // so the `unwrap` should never fail.
            let new_layout = Layout::array::<T>(new_cap).unwrap();
            (new_cap, new_layout)
        };

        // Ensure that the new allocation doesn't exceed `isize::MAX` bytes.
        assert!(
            new_layout.size() <= isize::MAX as usize,
            "Allocation too large"
        );

        let new_ptr = if cap == 0 {
            unsafe { alloc::alloc(new_layout) }
        } else {
            let old_ptr = buf.ptr.as_ptr().cast::<u8>();
            // Cannot use realloc here since it may drop the old pointer
            let new_ptr = unsafe { alloc::alloc(new_layout) };
            if NonNull::new(new_ptr.cast::<T>()).is_none() {
                alloc::handle_alloc_error(new_layout)
            }
            // This is fine since our elements are Copy
            let old_layout_len = Layout::array::<T>(len).unwrap();
            unsafe { std::ptr::copy_nonoverlapping(old_ptr, new_ptr, old_layout_len.size()) };
            new_ptr
        };

        // If allocation fails, `new_ptr` will be null, in which case we abort.
        self.repr = match NonNull::new(new_ptr.cast::<T>()) {
            Some(new_ptr) => {
                debug_assert_ne!(new_ptr, buf.ptr);
                CowVecRepr::Owned {
                    buf: Arc::new(RawBuf::new(new_ptr, len, new_cap)),
                }
            }
            None => alloc::handle_alloc_error(new_layout),
        };

        // Safety: we just assigned an owned repr in the previous statement
        //         to an exclusive reference
        match &self.repr {
            CowVecRepr::Snapshot { .. } => unsafe { unreachable_unchecked() },
            CowVecRepr::Owned { buf } => buf,
        }
    }
}

impl<T: Copy> Clone for CowVec<T> {
    #[inline]
    fn clone(&self) -> Self {
        let (buf, len) = match &self.repr {
            // Safety: Proven by the previous construction of the CowVec::Borrowed state.
            CowVecRepr::Snapshot { buf, len } => (buf.clone(), *len),
            // Safety: We are holding a shared ref, and the ref-counted buf
            //         can only be swapped if there is a exclusive ref.
            //         So, this access is safe.
            CowVecRepr::Owned { buf } => (buf.clone(), buf.len.load(Relaxed)),
        };
        Self {
            repr: CowVecRepr::Snapshot { buf, len },
        }
    }
}

impl<T> Deref for CowVec<T> {
    type Target = [T];

    #[inline]
    fn deref(&self) -> &Self::Target {
        let (ptr, len) = match &self.repr {
            // Safety: Proven by the previous construction of the CowVec::Borrowed state.
            CowVecRepr::Snapshot { buf, len } => (buf.as_ptr(), *len),
            // Safety: We are holding a shared ref, and the ref-counted buf
            //         can only be swapped if there is a exclusive ref.
            //         So, this access is safe.
            CowVecRepr::Owned { buf } => (buf.as_ptr(), buf.len.load(Relaxed)),
        };
        // Safety: see above
        unsafe { std::slice::from_raw_parts(ptr, len) }
    }
}

#[cfg(test)]
mod test {
    use std::sync::{Arc, Mutex};

    use super::CowVec;

    #[test]
    fn test_miri_push_and_access() {
        let mut arr = CowVec::new();
        for i in 0..10000 {
            arr.push(i);
        }
        for i in 0..10000 {
            assert_eq!(i, arr[i]);
        }
    }

    #[test]
    fn test_miri_push_and_concurrent_clone() {
        let arr = Arc::new(Mutex::new(CowVec::new()));
        let handle = std::thread::spawn({
            let arr = arr.clone();
            move || {
                for _ in 0..10 {
                    for i in 0..1000 {
                        arr.lock().unwrap().push(i);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
            }
        });

        while !handle.is_finished() {
            let mut arr = { arr.lock().unwrap().clone() };
            arr.push(arr.len());
            for i in arr.iter().copied() {
                assert_eq!(i, arr[i]);
            }
        }

        handle.join().unwrap();
    }

    #[test]
    fn test_miri_clone() {
        let mut arr = CowVec::new();
        for i in 0..10 {
            arr.push(i);
        }
        let mut cloned_arr = arr.clone();
        assert_eq!(arr.len(), cloned_arr.len());
        for i in 0..10 {
            assert_eq!(arr[i], cloned_arr[i]);
        }
        cloned_arr.push(11);
        arr.push(12);
        assert_ne!(arr[10], cloned_arr[10]);
    }

    #[test]
    fn test_miri_deref() {
        let mut arr = CowVec::new();
        for i in 0..10 {
            arr.push(i);
        }
        let slice: &[i32] = &arr;
        assert_eq!(slice.len(), arr.len());
        for i in 0..10 {
            assert_eq!(slice[i], arr[i]);
        }
    }
}
