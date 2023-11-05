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

enum CowVecRepr<T> {
    /// Snapshot form of the [`CowVec`]. Reads must be done using
    /// the saved length field.
    Borrowed {
        buf: Arc<AtomicAllocation<T>>,
        len: usize,
    },
    /// Owned form of the [`CowVec`]. Reads must be done using the
    /// atomic length.
    Owned {
        buf: Arc<AtomicAllocation<T>>,
    },
}

/// An allocation used in a [`SnapVec`].
pub(super) struct AtomicAllocation<T> {
    ptr: NonNull<T>,
    len: AtomicUsize,
    cap: usize,
}

impl<T> AtomicAllocation<T> {
    const fn empty() -> Self {
        Self::new(std::ptr::NonNull::dangling(), 0, 0)
    }

    const fn new(ptr: NonNull<T>, len: usize, cap: usize) -> Self {
        Self {
            ptr,
            len: AtomicUsize::new(len),
            cap,
        }
    }
}

impl<T> Deref for AtomicAllocation<T> {
    type Target = NonNull<T>;
    fn deref(&self) -> &Self::Target {
        &self.ptr
    }
}

impl<T> Drop for AtomicAllocation<T> {
    fn drop(&mut self) {
        let cap = self.cap;
        if cap != 0 {
            // Safety: we are the last owner, we can do a relaxed read of len
            unsafe {
                std::ptr::drop_in_place(std::ptr::slice_from_raw_parts_mut(
                    self.ptr.as_ptr(),
                    self.len.load(Relaxed),
                ))
            }
            unsafe {
                alloc::dealloc(
                    self.ptr.as_ptr() as *mut u8,
                    Layout::array::<T>(cap).unwrap(),
                );
            }
        }
    }
}

unsafe impl<T: Send> Send for AtomicAllocation<T> {}
unsafe impl<T: Sync> Sync for AtomicAllocation<T> {}

/// A copy-on-write vector for Copy only elements. Cloning this vector will give
/// a snapshot of the vector's content at the time of clone. The snapshot shares
/// the buffer with the original owning `CowVec` until it reallocates or until
/// the user attempts to mutably alter the data.
pub struct CowVec<T> {
    repr: CowVecRepr<T>,
}

impl<T> Debug for CowVec<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CowVec [..., {}]", self.len())
    }
}

impl<T> CowVec<T> {
    pub fn new() -> Self {
        assert!(std::mem::size_of::<T>() != 0);
        Self {
            repr: CowVecRepr::Owned {
                buf: Arc::new(AtomicAllocation::empty()),
            },
        }
    }

    pub fn len(&self) -> usize {
        // No matter what len we load, it will be valid since the length
        // is only incremented after the data is written.
        match &self.repr {
            CowVecRepr::Borrowed { len, .. } => *len,
            CowVecRepr::Owned { buf } => buf.len.load(Relaxed),
        }
    }
}

impl<T: Copy> CowVec<T> {
    pub fn new_one_elem(elem: T) -> Self {
        let mut vec = Self::new();
        vec.push(elem);
        vec
    }

    pub fn push(&mut self, elem: T) {
        let (buf, len) = match &self.repr {
            &CowVecRepr::Borrowed { len, .. } => (self.grow(), len),
            CowVecRepr::Owned { buf } => {
                let len = buf.len.load(Acquire);
                let cap = buf.cap;
                (if len == cap { self.grow() } else { buf }, len)
            }
        };

        unsafe {
            std::ptr::write_volatile(buf.ptr.as_ptr().add(len), elem);
        }

        // Can't fail, we'll OOM first.
        // There should be no other writers, but lets be safe.
        buf.len.store(len + 1, Release);
    }

    /// Grow will return a buffer that the caller can write to.
    fn grow(&mut self) -> &Arc<AtomicAllocation<T>> {
        let (buf, len) = match &self.repr {
            CowVecRepr::Borrowed { buf, len } => (buf, *len),
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
            let new_cap = 2 * cap;

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
            let old_ptr = buf.ptr.as_ptr() as *mut u8;
            // Cannot use realloc here since it may drop the old pointer
            unsafe {
                let old_layout = Layout::array::<T>(cap).unwrap();
                let new_ptr = alloc::alloc(new_layout);
                if NonNull::new(new_ptr as *mut T).is_none() {
                    alloc::handle_alloc_error(new_layout)
                }
                // This is fine since our elements are Copy
                std::ptr::copy_nonoverlapping(old_ptr, new_ptr, old_layout.size());
                new_ptr
            }
        };

        // If allocation fails, `new_ptr` will be null, in which case we abort.
        self.repr = match NonNull::new(new_ptr as *mut T) {
            Some(p) => {
                debug_assert_ne!(p, buf.ptr);
                CowVecRepr::Owned {
                    buf: Arc::new(AtomicAllocation::new(p, len, new_cap)),
                }
            }
            None => alloc::handle_alloc_error(new_layout),
        };

        // Safety: we just assigned an owned repr in the previous statement
        //         to an exclusive reference
        match &self.repr {
            CowVecRepr::Borrowed { .. } => unsafe { unreachable_unchecked() },
            CowVecRepr::Owned { buf } => buf,
        }
    }
}

impl<T: Copy> Clone for CowVec<T> {
    fn clone(&self) -> Self {
        let (buf, len) = match &self.repr {
            CowVecRepr::Borrowed { buf, len } => (buf.clone(), *len),
            CowVecRepr::Owned { buf } => {
                // Load using seqcst so it doesn't magically get reordered in the CPU
                // instruction buffer to after the Arc atomic clone (which uses relaxed)
                let len = buf.len.load(SeqCst);
                // Imagine that we clone the allocation information, atomically grow
                // the array, push an element, and then load the length. We would have
                // a length that's invalid for the old allocation. Therefore,
                // we must load the length before we clone, otherwise we can load a
                // bigger length than the capacity of the buffer we cloned.
                let buf = buf.clone();
                (buf, len)
            }
        };
        CowVec {
            repr: CowVecRepr::Borrowed { buf, len },
        }
    }
}

impl<T: Copy> Deref for CowVec<T> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        let (ptr, len) = match &self.repr {
            CowVecRepr::Borrowed { buf, len } => (buf.as_ptr(), *len),
            CowVecRepr::Owned { buf } => {
                let len = buf.len.load(SeqCst);
                (buf.as_ptr(), len)
            }
        };
        unsafe { std::slice::from_raw_parts(ptr, len) }
    }
}

mod test {
    #[test]
    fn simple() {
        use super::CowVec;
        let mut arr = CowVec::new();
        for i in 0..10000000 {
            arr.push(i);
        }
        for i in 0..10000000 {
            assert_eq!(i, arr[i]);
        }
    }
}
