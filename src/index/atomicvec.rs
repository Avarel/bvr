use std::{
    alloc::{self, Layout},
    ops::Deref,
    ptr::NonNull,
    sync::{
        atomic::{AtomicUsize, Ordering::SeqCst},
        Arc,
    }, fmt::Debug,
};

struct RawAtomicVec<T> {
    ptr: *mut T,
    len: AtomicUsize,
    cap: usize,
}

impl<T> RawAtomicVec<T> {
    const fn empty() -> Self {
        Self::new_component(std::ptr::NonNull::dangling().as_ptr(), 0, 0)
    }

    const fn new_component(ptr: *mut T, len: usize, cap: usize) -> Self {
        Self {
            ptr,
            len: AtomicUsize::new(len),
            cap,
        }
    }
}

impl<T> Deref for RawAtomicVec<T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        unsafe { std::slice::from_raw_parts(self.ptr, self.len.load(SeqCst)) }
    }
}

unsafe impl<T: Send> Send for RawAtomicVec<T> {}
unsafe impl<T: Sync> Sync for RawAtomicVec<T> {}

impl<T> Drop for RawAtomicVec<T> {
    fn drop(&mut self) {
        unsafe {
            std::ptr::drop_in_place(std::ptr::slice_from_raw_parts_mut(
                self.ptr,
                self.len.load(SeqCst),
            ))
        }
        unsafe {
            alloc::dealloc(self.ptr as *mut u8, Layout::array::<T>(self.cap).unwrap());
        }
    }
}

pub struct AtomicVecSnapshot<T>(Arc<RawAtomicVec<T>>);

impl<T> Deref for AtomicVecSnapshot<T> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}

pub struct AtomicVec<T> {
    inner: Arc<RawAtomicVec<T>>,
}

impl<T> Debug for AtomicVec<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "AtomicVec [..., {}]", self.inner.len())
    }
}

impl<T> AtomicVec<T> {
    pub fn new() -> Self {
        assert!(std::mem::size_of::<T>() != 0);
        Self {
            inner: Arc::new(RawAtomicVec::empty()),
        }
    }

    pub fn new_one_elem(elem: T) -> Self {
        let mut vec = Self::new();
        vec.push(elem);
        vec
    }

    pub fn push(&mut self, elem: T) {
        let len = self.inner.len.load(SeqCst);
        if len == self.inner.cap {
            self.grow();
        }

        unsafe {
            std::ptr::write_volatile(self.inner.ptr.add(len), elem);
        }

        // Can't fail, we'll OOM first.
        // There should be no other writers, but lets be safe.
        self.inner
            .len
            .compare_exchange(len, len + 1, SeqCst, SeqCst)
            .unwrap();
    }

    fn grow(&mut self) {
        let len = self.inner.len.load(SeqCst);
        let (new_cap, new_layout) = if self.inner.cap == 0 {
            (1, Layout::array::<T>(1).unwrap())
        } else {
            // This can't overflow since self.cap <= isize::MAX.
            let new_cap = 2 * self.inner.cap;

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

        let new_ptr = if self.inner.cap == 0 {
            unsafe { alloc::alloc(new_layout) }
        } else {
            let old_layout = Layout::array::<T>(self.inner.cap).unwrap();
            let old_ptr = self.inner.ptr as *mut u8;
            unsafe { alloc::realloc(old_ptr, old_layout, new_layout.size()) }
        };

        // If allocation fails, `new_ptr` will be null, in which case we abort.
        self.inner = match NonNull::new(new_ptr as *mut T) {
            Some(p) => Arc::new(RawAtomicVec::new_component(p.as_ptr(), len, new_cap)),
            None => alloc::handle_alloc_error(new_layout),
        };
    }

    pub fn snapshot(&self) -> AtomicVecSnapshot<T> {
        AtomicVecSnapshot(self.inner.clone())
    }
}

impl<T> Deref for AtomicVec<T> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        self.inner.deref()
    }
}