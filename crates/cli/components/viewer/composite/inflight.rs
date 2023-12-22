use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use bvr_core::err::Result;

use crate::components::viewer::masks::Mask;

use super::{CompleteComposite, IncompleteComposite};
#[doc(hidden)]
pub struct InflightCompositeImpl {
    inner: std::sync::Mutex<IncompleteComposite>,
    cache: std::sync::Mutex<Option<CompleteComposite>>,
    progress: AtomicU64,
}

impl InflightCompositeImpl {
    fn new() -> Arc<Self> {
        Arc::new(InflightCompositeImpl {
            inner: std::sync::Mutex::new(IncompleteComposite::new()),
            cache: std::sync::Mutex::new(None),
            progress: AtomicU64::new(0),
        })
    }

    fn compute(self: Arc<Self>, masks: Vec<Mask>) -> Result<()> {
        let len = masks.iter().filter_map(|v| v.len()).sum::<usize>();

        let mut masks = masks.into_iter().map(|v| (0, v)).collect::<Vec<_>>();

        while Arc::strong_count(&self) >= 2 {
            if let Some((offset, line_number)) = masks
                .iter_mut()
                .filter_map(|(offset, mask)| {
                    mask.translate_to_file_line(*offset).map(|ln| (offset, ln))
                })
                .min_by_key(|&(_, ln)| ln)
            {
                *offset += 1;

                let mut inner = self.inner.lock().unwrap();
                inner.add_line(line_number);

                let progress =
                    masks.iter().map(|(offset, _)| *offset).sum::<usize>() as f64 / len as f64;
                self.progress.store(
                    (progress * 100.0) as u64,
                    std::sync::atomic::Ordering::Relaxed,
                );
            } else if masks.iter().all(|(_, mask)| mask.is_complete()) {
                break;
            } else {
                continue;
            };
        }

        Ok(())
    }

    fn read<F, T>(&self, cb: F) -> T
    where
        F: FnOnce(&CompleteComposite) -> T,
    {
        match self.inner.try_lock() {
            Ok(index) => {
                let clone = index.inner.clone();
                let val = cb(&clone);
                *self.cache.lock().unwrap() = Some(clone);
                val
            }
            Err(_) => {
                let lock = self.cache.lock().unwrap();
                if let Some(v) = lock.as_ref() {
                    return cb(v);
                }
                drop(lock);

                let clone = self.inner.lock().unwrap().inner.clone();
                let val = cb(&clone);
                *self.cache.lock().unwrap() = Some(clone);
                val
            }
        }
    }
}

pub struct InflightCompositeRemote(Arc<InflightCompositeImpl>);

impl InflightCompositeRemote {
    pub fn compute(self, masks: Vec<Mask>) -> Result<()> {
        self.0.compute(masks)
    }
}

pub enum InflightCompositeProgress {
    Done,
    Partial(f64),
}

#[derive(Clone)]
pub enum InflightComposite {
    Incomplete(#[doc(hidden)] Arc<InflightCompositeImpl>),
    Complete(CompleteComposite),
}

impl InflightComposite {
    pub fn new() -> (Self, InflightCompositeRemote) {
        let inner = InflightCompositeImpl::new();
        (
            Self::Incomplete(inner.clone()),
            InflightCompositeRemote(inner),
        )
    }

    pub fn progress(&self) -> InflightCompositeProgress {
        match self {
            InflightComposite::Incomplete(inner) => {
                let progress =
                    inner.progress.load(std::sync::atomic::Ordering::Relaxed) as f64 / 100.0;
                InflightCompositeProgress::Partial(progress)
            }
            InflightComposite::Complete(_) => InflightCompositeProgress::Done,
        }
    }

    pub fn try_finalize(&mut self) -> bool {
        match self {
            Self::Incomplete(inner) => {
                match Arc::try_unwrap(std::mem::replace(
                    inner,
                    InflightCompositeImpl::new(),
                )) {
                    Ok(unwrapped) => {
                        *self = Self::Complete(unwrapped.inner.into_inner().unwrap().finish());
                        true
                    },
                    Err(old_inner) => {
                        *self = Self::Incomplete(old_inner);
                        false
                    }
                }
            }
            Self::Complete(_) => true,
        }
    }
}
