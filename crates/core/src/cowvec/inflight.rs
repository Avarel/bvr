//! Internal generalization of inflight structures.

use super::CowVec;
use std::sync::{atomic::AtomicBool, Arc, Mutex};

pub struct InflightCowVecWriter<T> {
    inflight: Mutex<CowVec<T>>,
    snapshot: Mutex<Option<CowVec<T>>>,
    complete: AtomicBool,
}

impl<T: Copy> InflightCowVecWriter<T> {
    pub fn new() -> Self {
        Self {
            inflight: Mutex::new(CowVec::new()),
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
        F: FnOnce(&mut CowVec<T>),
    {
        cb(&mut self.inflight.lock().unwrap())
    }

    pub fn read<F, R>(&self, cb: F) -> R
    where
        F: FnOnce(&CowVec<T>) -> R,
    {
        match self.inflight.try_lock() {
            Ok(index) => {
                let clone = index.clone();
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

                let clone = self.inflight.lock().unwrap().clone();
                let val = cb(&clone);
                *self.snapshot.lock().unwrap() = Some(clone);
                val
            }
        }
    }
}

#[derive(Clone)]
pub enum InflightCowVec<T: Copy> {
    Incomplete(#[doc(hidden)] Arc<InflightCowVecWriter<T>>),
    Complete(CowVec<T>),
}

impl<T: Copy> InflightCowVec<T> {
    pub fn try_finalize(&mut self) -> bool {
        match self {
            Self::Incomplete(inner) => {
                match Arc::try_unwrap(std::mem::replace(
                    inner,
                    Arc::new(InflightCowVecWriter::<T>::new()),
                )) {
                    Ok(unwrapped) => {
                        *self = Self::Complete(unwrapped.inflight.into_inner().unwrap());
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

    pub fn read<F, R>(&self, cb: F) -> R
    where
        F: FnOnce(&CowVec<T>) -> R,
    {
        match self {
            Self::Incomplete(inner) => inner.read(cb),
            Self::Complete(inner) => cb(inner),
        }
    }

    pub fn is_complete(&self) -> bool {
        match self {
            InflightCowVec::Incomplete(r) => r.complete.load(std::sync::atomic::Ordering::Relaxed),
            InflightCowVec::Complete(_) => true,
        }
    }
}
