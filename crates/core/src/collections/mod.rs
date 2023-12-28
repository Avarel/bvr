mod ftree;
pub mod indexset;

use arc_swap::ArcSwap;
use std::sync::Arc;

pub use indexset::BTreeSet;

pub struct LiveSet<T>
where
    T: Copy + Ord,
{
    inner: Arc<ArcSwap<BTreeSet<T>>>,
}

impl<T> LiveSet<T>
where
    T: Copy + Ord,
{
    pub fn new() -> Self {
        Self {
            inner: Arc::new(ArcSwap::new(Arc::new(BTreeSet::new()))),
        }
    }

    pub fn contains(&self, value: T) -> bool {
        self.inner.load().contains(value)
    }

    pub fn insert(&self, value: T) {
        self.inner.rcu(|set| Arc::new(set.insert_update(value)));
    }

    pub fn remove(&self, value: T) {
        self.inner.rcu(|set| Arc::new(set.remove_update(value)));
    }

    pub fn get_index(&self, i: usize) -> Option<T> {
        self.inner.load().get_index(i)
    }
}
