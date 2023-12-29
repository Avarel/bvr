use super::BTreeSet;
use arc_swap::ArcSwap;
use std::sync::Arc;

pub struct SharedIndexedSet<T>
where
    T: Copy + Ord,
{
    inner: ArcSwap<BTreeSet<T>>,
}

impl<T> SharedIndexedSet<T>
where
    T: Copy + Ord,
{
    pub fn new() -> Self {
        Self {
            inner: ArcSwap::new(Arc::new(BTreeSet::new())),
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

    pub fn get(&self, index: usize) -> Option<T> {
        self.inner.load().get_index(index)
    }

    pub fn last(&self) -> Option<T> {
        self.inner.load().last()
    }

    pub fn len(&self) -> usize {
        self.inner.load().len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.load().is_empty()
    }

    pub fn snapshot(&self) -> BTreeSet<T> {
        BTreeSet::clone(&self.inner.load())
    }

    pub fn find(&self, value: T) -> Result<usize, usize> {
        self.inner.load().find(value)
    }
}

impl<T> From<Vec<T>> for SharedIndexedSet<T>
where
    T: Copy + Ord,
{
    fn from(value: Vec<T>) -> Self {
        Self {
            inner: ArcSwap::new(Arc::new(value.into_iter().collect())),
        }
    }
}
