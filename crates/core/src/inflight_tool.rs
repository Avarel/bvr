//! Internal generalization of inflight structures.

use std::sync::{atomic::AtomicBool, Arc, Mutex};

pub trait Inflightable: Sized {
    /// The incomplete underlying data.
    type Incomplete: Default;

    /// Finish the incomplete data and create a complete data.
    fn finish(inner: Self::Incomplete) -> Self;

    /// Create a snapshot of the incomplete data.
    ///
    /// Ideally, this operation should be *extremely* cheap to reduce
    /// the amount of time the internal lock is held.
    fn snapshot(inner: &Self::Incomplete) -> Self;
}

pub struct InflightImpl<I>
where
    I: Inflightable,
{
    inflight: Mutex<I::Incomplete>,
    snapshot: Mutex<Option<I>>,
    complete: AtomicBool,
}

impl<I> InflightImpl<I>
where
    I: Inflightable,
{
    pub fn new() -> Self {
        Self {
            inflight: Mutex::new(I::Incomplete::default()),
            snapshot: Mutex::new(None),
            complete: AtomicBool::new(false),
        }
    }

    pub fn mark_complete(&self) {
        self.complete
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn write<F>(&self, cb: F)
    where
        F: FnOnce(&mut I::Incomplete),
    {
        cb(&mut self.inflight.lock().unwrap())
    }

    pub fn read<F, T>(&self, cb: F) -> T
    where
        F: FnOnce(&I) -> T,
    {
        match self.inflight.try_lock() {
            Ok(index) => {
                let clone = I::snapshot(&index);
                let val = cb(&clone);
                *self.snapshot.lock().unwrap() = Some(clone);
                val
            }
            Err(_) => {
                let lock = self.snapshot.lock().unwrap();
                if let Some(v) = lock.as_ref() {
                    return cb(v);
                }
                drop(lock);

                let clone = I::snapshot(&self.inflight.lock().unwrap());
                let val = cb(&clone);
                *self.snapshot.lock().unwrap() = Some(clone);
                val
            }
        }
    }
}

#[derive(Clone)]
pub enum Inflight<I>
where
    I: Inflightable,
{
    Incomplete(#[doc(hidden)] Arc<InflightImpl<I>>),
    Complete(I),
}

impl<I> Inflight<I>
where
    I: Inflightable,
{
    pub fn try_finalize(&mut self) -> bool {
        match self {
            Self::Incomplete(inner) => {
                match Arc::try_unwrap(std::mem::replace(inner, Arc::new(InflightImpl::<I>::new())))
                {
                    Ok(unwrapped) => {
                        *self = Self::Complete(I::finish(unwrapped.inflight.into_inner().unwrap()));
                        true
                    }
                    Err(old_inner) => {
                        *self = Self::Incomplete(old_inner);
                        false
                    }
                }
            }
            Self::Complete(_) => true,
        }
    }

    pub fn is_complete(&self) -> bool {
        match self {
            Inflight::Incomplete(r) => r.complete.load(std::sync::atomic::Ordering::Relaxed),
            Inflight::Complete(_) => true,
        }
    }

    pub fn unwrap(mut self) -> I {
        match self {
            Self::Incomplete { .. } => {
                if self.try_finalize() {
                    self.unwrap()
                } else {
                    panic!("indexing is incomplete")
                }
            }
            Self::Complete(inner) => inner,
        }
    }
}
