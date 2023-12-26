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

use arc_swap::ArcSwap;

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

pub struct CowVecWriter<T> {
    buf: Arc<ArcSwap<RawBuf<T>>>,
}

impl<T> CowVecWriter<T>
where
    T: Copy,
{
    /// Appends an element to the back of this collection.
    ///
    /// If the collection is in a borrowed state, it will copy the data
    /// underlying the borrowed state and transition to an owned state.
    pub fn push(&mut self, elem: T) {
        let buf = self.buf.load();
        let len = buf.len.load(Ordering::SeqCst);
        let cap = buf.cap;
        if len == cap {
            // Safety: If this runs, then buf will no longer be borrowed from
            push_inner(&self.grow(), len, elem)
        } else {
            push_inner(&buf, len, elem)
        }

        #[inline(always)]
        fn push_inner<T: Copy>(buf: &RawBuf<T>, len: usize, elem: T) {
            unsafe { std::ptr::write_volatile(buf.ptr.as_ptr().add(len), elem) }

            // Can't fail, we'll OOM first.
            // There should be no other writers, but lets be safe.
            buf.len.store(len + 1, Ordering::Release);
        }
    }

    /// Grow will return a buffer that the caller can write to.
    fn grow(&mut self) -> Arc<RawBuf<T>> {
        let buf = self.buf.load();
        let len = buf.len.load(Ordering::SeqCst);
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
        match NonNull::new(new_ptr.cast::<T>()) {
            Some(new_ptr) => {
                debug_assert_ne!(new_ptr, buf.ptr);
                let ret = Arc::new(RawBuf::new(new_ptr, len, new_cap));
                self.buf.store(ret.clone());
                ret
            }
            None => alloc::handle_alloc_error(new_layout),
        }
    }
}

#[derive(Clone)]
pub struct CowVec<T> {
    buf: Arc<ArcSwap<RawBuf<T>>>,
}

impl<T> CowVec<T> {
    /// Constructs a new, empty `CowVec<T>`.
    ///
    /// The vector will not allocate until elements are pushed onto it.
    #[inline]
    pub fn new() -> (Self, CowVecWriter<T>) {
        assert!(std::mem::size_of::<T>() != 0);
        let buf = Arc::new(ArcSwap::new(Arc::new(RawBuf::empty())));
        (Self { buf: buf.clone() }, CowVecWriter { buf })
    }

    pub fn len(&self) -> usize {
        self.read(|slice| slice.len())
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn read<F, R>(&self, cb: F) -> R
    where
        F: FnOnce(&[T]) -> R,
    {
        let buf = self.buf.load();
        let len = buf.len.load(Ordering::SeqCst);
        cb(unsafe { std::slice::from_raw_parts(buf.as_ptr(), len) })
    }

    pub fn snapshot(&self) -> CowVecSnapshot<'_, T> {
        CowVecSnapshot {
            buf: self.buf.load_full(),
            _phantom: PhantomData,
        }
    }
}

impl<T> CowVec<T>
where
    T: Copy,
{
    pub fn get(&self, index: usize) -> Option<T> {
        self.read(|slice| slice.get(index).copied())
    }
}

struct CowVecSnapshot<'a, T> {
    buf: Arc<RawBuf<T>>,
    _phantom: PhantomData<&'a ()>,
}

impl<T> CowVecSnapshot<'_, T>
where
    T: Copy,
{
    pub fn get(&self, index: usize) -> Option<T> {
        self.deref().get(index).copied()
    }
}

impl<T> Deref for CowVecSnapshot<'_, T> {
    type Target = [T];

    #[inline]
    fn deref(&self) -> &Self::Target {
        let buf = &self.buf;
        let len = buf.len.load(Ordering::SeqCst);
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
    fn test_miri_push_and_concurrent_clone() {
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
}
