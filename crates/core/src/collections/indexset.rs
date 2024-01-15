// Persistent B-tree index set implementation, specialized for Copy types.
// Based on `indexset` crate.

const DEFAULT_INNER_SIZE: usize = 1 << 10;
const CUTOFF_RATIO: usize = 2;
const DEFAULT_CUTOFF: usize = DEFAULT_INNER_SIZE / CUTOFF_RATIO;

use super::ftree::FenwickTree;
use std::cmp::Ordering;

#[derive(Clone, Debug, PartialEq)]
struct Node<T> {
    inner: Vec<T>,
    max: Option<T>,
    iterations: usize,
}

impl<T: Ord + Copy> PartialOrd for Node<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.max.partial_cmp(&other.max)
    }
}

impl<T: Ord + Copy> Default for Node<T> {
    fn default() -> Self {
        Self {
            inner: Vec::new(),
            max: None,
            iterations: 10,
        }
    }
}

impl<T: Ord + Copy> Node<T> {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Vec::with_capacity(capacity),
            iterations: capacity.ilog2() as usize,
            ..Default::default()
        }
    }

    pub fn get(&self, index: usize) -> Option<T> {
        self.inner.get(index).copied()
    }

    pub fn split_off(&mut self, cutoff: usize) -> Self {
        let latter_inner = self.inner.split_off(cutoff);

        self.max = self.inner.last().copied();

        let latter_inner_max = latter_inner.last().copied();
        Self {
            inner: latter_inner,
            max: latter_inner_max,
            iterations: self.iterations,
        }
    }

    pub fn halve(&mut self) -> Self {
        self.split_off(DEFAULT_CUTOFF)
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn insert(&mut self, value: T) -> bool {
        fn search<T: PartialOrd>(
            haystack: &[T],
            needle: &T,
            iterations: usize,
        ) -> Result<usize, usize> {
            let mut left = 0;
            let mut right = haystack.len();
            for _ in 0..iterations {
                if left >= right {
                    break;
                }

                let mid = left + (right - left) / 2;

                let mid_value = unsafe { haystack.get_unchecked(mid) };

                if mid_value < needle {
                    left = mid + 1;
                } else if mid_value > needle {
                    right = mid;
                } else {
                    return Ok(mid);
                }
            }

            Err(left)
        }

        match search(&self.inner, &value, self.iterations) {
            Ok(_) => return false,
            Err(idx) => {
                let some_value = Some(&value);
                if some_value > self.max.as_ref() {
                    self.max = some_value.cloned()
                }

                self.inner.insert(idx, value);
            }
        }

        true
    }

    pub fn delete(&mut self, index: usize) -> T {
        self.inner.remove(index)
    }
}

/// An ordered set based on a B-Tree.
///
/// It is a logic error for an item to be modified in such a way that the item's ordering relative
/// to any other item, as determined by the [`Ord`] trait, changes while it is in the set. This is
/// normally only possible through [`Cell`], [`RefCell`], global state, I/O, or unsafe code.
/// The behavior resulting from such a logic error is not specified, but will be encapsulated to the
/// `BTreeSet` that observed the logic error and not result in undefined behavior. This could
/// include panics, incorrect results, aborts, memory leaks, and non-termination.
///
/// Iterators returned by [`BTreeSet::iter`] produce their items in order, and take worst-case
/// logarithmic and amortized constant time per item returned.
///
/// [`Cell`]: core::cell::Cell
/// [`RefCell`]: core::cell::RefCell
///
///
/// A `BTreeSet` with a known list of items can be initialized from an array:
///
/// ```
/// use bvr_core::collections::indexset::BTreeSet;
///
/// let set = BTreeSet::from_iter([1, 2, 3]);
/// ```
#[derive(Clone)]
pub struct BTreeSet<T> {
    inner: im::Vector<Node<T>>,
    index: FenwickTree<usize>,
    node_capacity: usize,
    len: usize,
}

impl<T: Copy + Ord> BTreeSet<T> {
    /// Makes a new, empty `BTreeSet` with maximum node size 1024. Allocates one vec of capacity 1024.
    ///
    /// Note that this does not mean that the maximum number of items is 1024.
    ///
    /// In case you would like to make a tree with a different maximum node size, use the
    /// `with_maximum_node_size` method.
    ///
    /// # Examples
    ///
    /// ```
    /// # #![allow(unused_mut)]
    /// use bvr_core::collections::indexset::BTreeSet;
    ///
    /// let mut set: BTreeSet<i32> = BTreeSet::new();
    /// ```
    pub fn new() -> Self {
        Self {
            ..Default::default()
        }
    }

    /// Clears the set, removing all elements.
    ///
    /// # Examples
    ///
    /// ```
    /// use bvr_core::collections::indexset::BTreeSet;
    ///
    /// let mut v = BTreeSet::new();
    /// v.insert(1);
    /// v.clear();
    /// assert!(v.is_empty());
    /// ```
    pub fn clear(&mut self) {
        self.inner = im::vector![Node::new(self.node_capacity)];
        self.index = FenwickTree::from_iter(vec![0]);
        self.len = 0;
    }

    fn locate_node(&self, value: T) -> usize {
        fn partition_point<T: Clone, P>(v: &im::Vector<T>, mut pred: P) -> usize
        where
            P: FnMut(&T) -> bool,
        {
            v.binary_search_by(|x| {
                if pred(x) {
                    Ordering::Less
                } else {
                    Ordering::Greater
                }
            })
            .unwrap_or_else(|i| i)
        }

        let mut node_idx = partition_point(&self.inner, |node| {
            if let Some(max) = node.max {
                return max < value;
            };

            false
        });

        // When value is greater than all elements inside inner[node_idx], then len
        // of inner[node_idx], which is not a valid place for insertion, is returned. It will
        // never return less than 0, so it is only necessary to check whether it is out of bounds
        // from the right
        if self.inner.get(node_idx).is_none() {
            node_idx -= 1
        }

        node_idx
    }

    fn locate_value(&self, value: T) -> (usize, usize) {
        let node_idx = self.locate_node(value);
        let position_within_node = self.inner[node_idx]
            .inner
            .partition_point(|&item| item < value);

        (node_idx, position_within_node)
    }

    fn locate_ith(&self, idx: usize) -> Option<(usize, usize)> {
        if idx >= self.len() {
            return None;
        }
        let mut node_index = self.index.index_of(idx);
        let mut offset = 0;

        if node_index != 0 {
            offset = self.index.prefix_sum(node_index, 0);
        }

        let mut position_within_node = idx - offset;
        if let Some(node) = self.inner.get(node_index) {
            if position_within_node > node.len() - 1 {
                node_index += 1;
                position_within_node = 0;
            }
        }

        Some((node_index, position_within_node))
    }

    /// Returns a reference to the element in the i-th position of the set, if any.
    ///
    /// # Examples
    ///
    /// ```
    /// use bvr_core::collections::indexset::BTreeSet;
    ///
    /// let set = BTreeSet::from_iter([1, 2, 3]);
    /// assert_eq!(set.get_index(0), Some(1));
    /// assert_eq!(set.get_index(2), Some(3));
    /// assert_eq!(set.get_index(4), None);
    /// ```
    pub fn get_index(&self, idx: usize) -> Option<T> {
        let (node_idx, position_within_node) = self.locate_ith(idx)?;
        if let Some(candidate_node) = self.inner.get(node_idx) {
            return candidate_node.get(position_within_node);
        }

        None
    }

    pub fn find(&self, value: T) -> Result<usize, usize> {
        let (node_idx, position_within_node) = self.locate_value(value);
        if let Some(candidate_node) = self.inner.get(node_idx) {
            if let Some(candidate_value) = candidate_node.get(position_within_node) {
                if value == candidate_value {
                    return Ok(self.index.prefix_sum(node_idx, 0) + position_within_node);
                }
            }
        }

        Err(self.index.prefix_sum(node_idx, 0) + position_within_node)
    }

    /// Returns a reference to the element in the set, if any, that is equal to
    /// the value.
    ///
    /// # Examples
    ///
    /// ```
    /// use bvr_core::collections::indexset::BTreeSet;
    ///
    /// let set = BTreeSet::from([1, 2, 3]);
    /// assert_eq!(set.get(2), Some(2));
    /// assert_eq!(set.get(4), None);
    /// ```
    pub fn get(&self, value: T) -> Option<T> {
        let (node_idx, position_within_node) = self.locate_value(value);
        if let Some(candidate_node) = self.inner.get(node_idx) {
            return candidate_node.get(position_within_node);
        }

        None
    }
    /// Returns a reference to the first element in the set, if any, that is not less than the
    /// input.
    ///
    /// # Examples
    ///
    /// ```
    /// use bvr_core::collections::indexset::BTreeSet;
    ///
    /// let set = BTreeSet::from_iter([1, 2, 3, 5]);
    /// assert_eq!(set.lower_bound(2), Some(2));
    /// assert_eq!(set.lower_bound(4), Some(5));
    /// ```
    pub fn lower_bound(&self, value: T) -> Option<T> {
        let (node_idx, position_within_node) = self.locate_value(value);
        if let Some(candidate_node) = self.inner.get(node_idx) {
            return candidate_node.get(position_within_node);
        }

        None
    }

    /// Returns a reference to the first element in the set, if any, that is strictly less than the
    /// input.
    ///
    /// # Examples
    ///
    /// ```
    /// use bvr_core::collections::indexset::BTreeSet;
    ///
    /// let set = BTreeSet::from_iter([1, 2, 3, 5]);
    /// assert_eq!(set.upper_bound_exclusive(1), None);
    /// assert_eq!(set.upper_bound_exclusive(2), Some(1));
    /// assert_eq!(set.upper_bound_exclusive(4), Some(3));
    /// ```
    pub fn upper_bound_exclusive(&self, value: T) -> Option<T> {
        let (node_idx, position_within_node) = self.locate_value(value);
        if position_within_node != 0 {
            if let Some(candidate_node) = self.inner.get(node_idx) {
                return candidate_node.get(position_within_node - 1);
            }
        } else if node_idx != 0 {
            if let Some(candidate_node) = self.inner.get(node_idx - 1) {
                return candidate_node.inner.last().copied();
            }
        }
        None
    }

    /// Returns the number of elements in the set.
    ///
    /// # Examples
    ///
    /// ```
    /// use bvr_core::collections::indexset::BTreeSet;
    ///
    /// let mut v = BTreeSet::new();
    /// assert_eq!(v.len(), 0);
    /// v.insert(1);
    /// assert_eq!(v.len(), 1);
    /// ```
    pub fn len(&self) -> usize {
        self.len
    }
    /// Adds a value to the set.
    ///
    /// Returns whether the value was newly inserted. That is:
    ///
    /// - If the set did not previously contain an equal value, `true` is
    ///   returned.
    /// - If the set already contained an equal value, `false` is returned, and
    ///   the entry is not updated.
    ///
    /// See the [module-level documentation] for more.
    ///
    /// [module-level documentation]: index.html#insert-and-complex-keys
    ///
    /// # Examples
    ///
    /// ```
    /// use bvr_core::collections::indexset::BTreeSet;
    ///
    /// let mut set = BTreeSet::new();
    ///
    /// assert_eq!(set.insert(2), true);
    /// assert_eq!(set.insert(2), false);
    /// assert_eq!(set.len(), 1);
    /// ```
    pub fn insert(&mut self, value: T) -> bool {
        let node_idx = self.locate_node(value);
        if self.inner[node_idx].len() == DEFAULT_INNER_SIZE {
            let new_node = self.inner[node_idx].halve();
            // Get the minimum
            let new_node_min = new_node.inner[0];
            // Insert the new node
            self.inner.insert(node_idx + 1, new_node);
            let insert_node_idx = if value < new_node_min {
                node_idx
            } else {
                node_idx + 1
            };
            if self.inner[insert_node_idx].insert(value) {
                // Reconstruct the index after the new node insert.
                self.index = FenwickTree::from_iter(self.inner.iter().map(|node| node.len()));
                self.len += 1;
                true
            } else {
                false
            }
        } else if self.inner[node_idx].insert(value) {
            self.index.add_at(node_idx, 1);
            self.len += 1;
            true
        } else {
            false
        }
    }

    /// Adds a value to the set.
    ///
    /// Returns a copy of the set with the value present.
    pub fn insert_update(&self, value: T) -> Self {
        let mut next = self.clone();
        next.insert(value);
        next
    }

    /// Returns `true` if the set contains an element equal to the value.
    ///
    /// # Examples
    ///
    /// ```
    /// use bvr_core::collections::indexset::BTreeSet;
    ///
    /// let set = BTreeSet::from_iter([1, 2, 3]);
    /// assert_eq!(set.contains(1), true);
    /// assert_eq!(set.contains(4), false);
    /// ```
    pub fn contains(&self, value: T) -> bool {
        let (node_idx, position_within_node) = self.locate_value(value);
        if let Some(candidate_node) = self.inner.get(node_idx) {
            if let Some(candidate_value) = candidate_node.get(position_within_node) {
                return value == candidate_value;
            }
        }

        false
    }

    fn delete_at(&mut self, node_idx: usize, position_within_node: usize) -> T {
        let removal = self.inner[node_idx].delete(position_within_node);

        let mut decrease_length = false;
        // check whether the node has to be deleted
        if self.inner[node_idx].len() == 0 {
            // delete it as long as it is not the last remaining node
            if self.inner.len() > 1 {
                self.inner.remove(node_idx);
                self.len -= 1;
                self.index = FenwickTree::from_iter(self.inner.iter().map(|node| node.len()));
            } else {
                decrease_length = true;
            }
        } else {
            decrease_length = true;
        }

        if decrease_length {
            self.index.sub_at(node_idx, 1);
            self.len -= 1;
        }

        removal
    }

    fn delete(&mut self, value: T) -> (Option<T>, bool) {
        let mut removed = false;
        let mut removal = None;
        let (node_idx, position_within_node) = self.locate_value(value);
        if let Some(candidate_node) = self.inner.get(node_idx) {
            if let Some(candidate_value) = candidate_node.get(position_within_node) {
                if value == candidate_value {
                    removal = Some(self.delete_at(node_idx, position_within_node));
                    removed = true;
                }
            }
        }

        (removal, removed)
    }

    /// If the set contains an element equal to the value, removes it from the
    /// set and drops it. Returns whether such an element was present.
    ///
    /// # Examples
    ///
    /// ```
    /// use bvr_core::collections::indexset::BTreeSet;
    ///
    /// let mut set = BTreeSet::new();
    ///
    /// set.insert(2);
    /// assert_eq!(set.remove(2), true);
    /// assert_eq!(set.remove(2), false);
    /// ```
    pub fn remove(&mut self, value: T) -> bool {
        self.delete(value).1
    }

    /// If the set contains an element equal to the value, removes it from the
    /// set and drops it. Returns a copy of the set with the element removed.
    ///
    /// # Examples
    ///
    /// ```
    /// use bvr_core::collections::indexset::BTreeSet;
    ///
    /// let mut set = BTreeSet::new();
    ///
    /// set.insert(2);
    /// assert_eq!(set.remove(2), true);
    /// assert_eq!(set.remove(2), false);
    /// ```
    pub fn remove_update(&self, value: T) -> Self {
        let mut next = self.clone();
        next.remove(value);
        next
    }

    /// Returns a reference to the first element in the set, if any.
    /// This element is always the minimum of all elements in the set.
    ///
    /// # Examples
    ///
    /// Basic usage:
    ///
    /// ```
    /// use bvr_core::collections::indexset::BTreeSet;
    ///
    /// let mut set = BTreeSet::new();
    /// assert_eq!(set.first(), None);
    /// set.insert(1);
    /// assert_eq!(set.first(), Some(1));
    /// set.insert(2);
    /// assert_eq!(set.first(), Some(1));
    /// ```
    pub fn first(&self) -> Option<T> {
        if let Some(candidate_node) = self.inner.get(0) {
            return candidate_node.get(0);
        }

        None
    }
    /// Returns a reference to the last element in the set, if any.
    /// This element is always the maximum of all elements in the set.
    ///
    /// # Examples
    ///
    /// Basic usage:
    ///
    /// ```
    /// use bvr_core::collections::indexset::BTreeSet;
    ///
    /// let mut set = BTreeSet::new();
    /// assert_eq!(set.last(), None);
    /// set.insert(1);
    /// assert_eq!(set.last(), Some(1));
    /// set.insert(2);
    /// assert_eq!(set.last(), Some(2));
    /// ```
    pub fn last(&self) -> Option<T> {
        if let Some(candidate_node) = self.inner.get(self.inner.len() - 1) {
            if candidate_node.len() > 0 {
                return candidate_node.get(candidate_node.len() - 1);
            }
        }

        None
    }
    /// Removes the first element from the set and returns it, if any.
    /// The first element is always the minimum element in the set.
    ///
    /// # Examples
    ///
    /// ```
    /// use bvr_core::collections::indexset::BTreeSet;
    ///
    /// let mut set = BTreeSet::new();
    ///
    /// set.insert(1);
    /// while let Some(n) = set.pop_first() {
    ///     assert_eq!(n, 1);
    /// }
    /// assert!(set.is_empty());
    /// ```
    pub fn pop_first(&mut self) -> Option<T> {
        let (first_node_idx, first_position_within_node) = (0, 0);
        if let Some(candidate_node) = self.inner.get(first_node_idx) {
            if candidate_node.get(first_position_within_node).is_some() {
                return Some(self.delete_at(first_node_idx, first_position_within_node));
            }
        }

        None
    }
    /// Removes the i-th element from the set and returns it, if any.
    ///
    /// # Examples
    ///
    /// ```
    /// use bvr_core::collections::indexset::BTreeSet;
    ///
    /// let mut set = BTreeSet::new();
    ///
    /// set.insert(1);
    /// set.insert(2);
    /// assert_eq!(set.pop_index(0), 1);
    /// assert_eq!(set.pop_index(0), 2);
    /// assert!(set.is_empty());
    /// ```
    pub fn pop_index(&mut self, idx: usize) -> T {
        let (node_idx, position_within_node) = self.locate_ith(idx).expect("valid index");

        self.delete_at(node_idx, position_within_node)
    }
    /// Removes the last element from the set and returns it, if any.
    /// The last element is always the maximum element in the set.
    ///
    /// # Examples
    ///
    /// ```
    /// use bvr_core::collections::indexset::BTreeSet;
    ///
    /// let mut set = BTreeSet::new();
    ///
    /// set.insert(1);
    /// while let Some(n) = set.pop_last() {
    ///     assert_eq!(n, 1);
    /// }
    /// assert!(set.is_empty());
    /// ```
    pub fn pop_last(&mut self) -> Option<T> {
        let last_node_idx = self.inner.len() - 1;
        let mut last_position_within_node = self.inner[last_node_idx].inner.len();
        last_position_within_node = last_position_within_node.saturating_sub(1);

        if let Some(candidate_node) = self.inner.get(last_node_idx) {
            if candidate_node.get(last_position_within_node).is_some() {
                return Some(self.delete_at(last_node_idx, last_position_within_node));
            }
        }

        None
    }
    /// Returns `true` if the set contains no elements.
    ///
    /// # Examples
    ///
    /// ```
    /// use bvr_core::collections::indexset::BTreeSet;
    ///
    /// let mut v = BTreeSet::new();
    /// assert!(v.is_empty());
    /// v.insert(1);
    /// assert!(!v.is_empty());
    /// ```
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Splits the collection into two at the value. Returns a new collection
    /// with all elements greater than or equal to the value.
    ///
    /// # Examples
    ///
    /// Basic usage:
    ///
    /// ```
    /// use bvr_core::collections::indexset::BTreeSet;
    ///
    /// let mut a = BTreeSet::new();
    /// a.insert(1);
    /// a.insert(2);
    /// a.insert(3);
    /// a.insert(17);
    /// a.insert(41);
    ///
    /// let b = a.split_off(3);
    ///
    /// assert_eq!(a.len(), 2);
    /// assert_eq!(b.len(), 3);
    ///
    /// assert!(a.contains(1));
    /// assert!(a.contains(2));
    ///
    /// assert!(b.contains(3));
    /// assert!(b.contains(17));
    /// assert!(b.contains(41));
    /// ```
    pub fn split_off(&mut self, value: T) -> Self {
        let (node_idx, position_within_node) = self.locate_value(value);
        let first_node = self.inner[node_idx].split_off(position_within_node);
        let mut remaining_nodes = im::Vector::new();
        while self.inner.len() > node_idx + 1 {
            remaining_nodes.push_front(self.inner.pop_back().unwrap());
        }
        remaining_nodes.insert(0, first_node);
        let mut latter_half = BTreeSet::default();
        latter_half.len = remaining_nodes.iter().map(|node| node.len()).sum();
        latter_half.inner = remaining_nodes;
        latter_half.index = FenwickTree::from_iter(latter_half.inner.iter().map(|node| node.len()));

        if self.inner[node_idx].len() == 0 && self.inner.len() > 1 {
            self.inner.remove(node_idx);
        }

        self.index = FenwickTree::from_iter(self.inner.iter().map(|node| node.len()));
        self.len = self.inner.iter().map(|node| node.len()).sum();

        latter_half
    }
    /// Moves all elements from `other` into `self`, leaving `other` empty.
    ///
    /// # Examples
    ///
    /// ```
    /// use bvr_core::collections::indexset::BTreeSet;
    ///
    /// let mut a = BTreeSet::new();
    /// a.insert(1);
    /// a.insert(2);
    /// a.insert(3);
    ///
    /// let mut b = BTreeSet::new();
    /// b.insert(3);
    /// b.insert(4);
    /// b.insert(5);
    ///
    /// a.append(&mut b);
    ///
    /// assert_eq!(a.len(), 5);
    /// assert_eq!(b.len(), 0);
    ///
    /// assert!(a.contains(1));
    /// assert!(a.contains(2));
    /// assert!(a.contains(3));
    /// assert!(a.contains(4));
    /// assert!(a.contains(5));
    /// ```
    pub fn append(&mut self, other: &mut Self) {
        while let Some(value) = other.pop_first() {
            self.insert(value);
        }
    }

    /// Returns the position in which the given element would fall in the already-existing sorted
    /// order.
    ///
    /// # Examples
    ///
    /// ```
    /// use bvr_core::collections::indexset::BTreeSet;
    ///
    /// let set = BTreeSet::from_iter([1, 2, 3]);
    /// assert_eq!(set.rank(1), 0);
    /// assert_eq!(set.rank(3), 2);
    /// assert_eq!(set.rank(4), 3);
    /// assert_eq!(set.rank(100), 3);
    /// ```
    pub fn rank(&self, value: T) -> usize {
        let (node_idx, position_within_node) = self.locate_value(value);

        let offset = self.index.prefix_sum(node_idx, 0);

        offset + position_within_node
    }

    /// Gets an iterator that visits the elements in the `BTreeSet` in ascending
    /// order.
    ///
    /// # Examples
    ///
    /// ```
    /// use bvr_core::collections::indexset::BTreeSet;
    ///
    /// let set = BTreeSet::from_iter([1, 2, 3]);
    /// let mut set_iter = set.iter();
    /// assert_eq!(set_iter.next(), Some(1));
    /// assert_eq!(set_iter.next(), Some(2));
    /// assert_eq!(set_iter.next(), Some(3));
    /// assert_eq!(set_iter.next(), None);
    /// ```
    ///
    /// Values returned by the iterator are returned in ascending order:
    ///
    /// ```
    /// use bvr_core::collections::indexset::BTreeSet;
    ///
    /// let set = BTreeSet::from_iter([3, 1, 2]);
    /// let mut set_iter = set.iter();
    /// assert_eq!(set_iter.next(), Some(1));
    /// assert_eq!(set_iter.next(), Some(2));
    /// assert_eq!(set_iter.next(), Some(3));
    /// assert_eq!(set_iter.next(), None);
    /// ```
    pub fn iter(&self) -> Iter<T> {
        return Iter::new(self);
    }
}

impl<T> FromIterator<T> for BTreeSet<T>
where
    T: Ord + Copy,
{
    fn from_iter<K: IntoIterator<Item = T>>(iter: K) -> Self {
        let mut btree = BTreeSet::new();
        iter.into_iter().for_each(|item| {
            btree.insert(item);
        });

        btree
    }
}

impl<T, const N: usize> From<[T; N]> for BTreeSet<T>
where
    T: Ord + Copy,
{
    fn from(value: [T; N]) -> Self {
        let mut btree: BTreeSet<T> = Default::default();

        value.into_iter().for_each(|item| {
            btree.insert(item);
        });

        btree
    }
}

impl<T> Default for BTreeSet<T>
where
    T: Copy + Ord,
{
    fn default() -> Self {
        let node_capacity = DEFAULT_INNER_SIZE;

        Self {
            inner: im::vector![Node::new(node_capacity)],
            index: FenwickTree::from_iter(vec![0]),
            node_capacity,
            len: 0,
        }
    }
}

/// An iterator over the items of a `BTreeSet`.
///
/// This `struct` is created by the [`iter`] method on [`BTreeSet`].
/// See its documentation for more.
///
/// [`iter`]: BTreeSet::iter
pub struct Iter<'a, T>
where
    T: Copy + Ord,
{
    btree: &'a BTreeSet<T>,
    current_front_node_idx: usize,
    current_front_idx: usize,
    current_back_node_idx: usize,
    current_back_idx: usize,
    current_front_iterator: std::slice::Iter<'a, T>,
    current_back_iterator: std::slice::Iter<'a, T>,
}

impl<'a, T> Iter<'a, T>
where
    T: Copy + Ord,
{
    pub fn new(btree: &'a BTreeSet<T>) -> Self {
        return Self {
            btree,
            current_front_node_idx: 0,
            current_front_idx: 0,
            current_back_node_idx: btree.inner.len() - 1,
            current_back_idx: btree.len(),
            current_front_iterator: btree.inner[0].inner.iter(),
            current_back_iterator: btree.inner[btree.inner.len() - 1].inner.iter(),
        };
    }
}

impl<'a, T> Iterator for Iter<'a, T>
where
    T: Copy + Ord,
{
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        if self.current_front_idx == self.current_back_idx {
            return None;
        }
        return if let Some(&value) = self.current_front_iterator.next() {
            self.current_front_idx += 1;
            Some(value)
        } else {
            if self.current_front_node_idx == self.btree.inner.len() - 1 {
                return None;
            }
            self.current_front_node_idx += 1;
            self.current_front_iterator =
                self.btree.inner[self.current_front_node_idx].inner.iter();
            if let Some(&value) = self.current_front_iterator.next() {
                return Some(value);
            }

            None
        };
    }
}

impl<'a, T> DoubleEndedIterator for Iter<'a, T>
where
    T: Copy + Ord,
{
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.current_front_idx == self.current_back_idx {
            return None;
        }
        return if let Some(&value) = self.current_back_iterator.next_back() {
            self.current_back_idx -= 1;
            Some(value)
        } else {
            if self.current_back_node_idx == 0 {
                return None;
            };
            self.current_back_node_idx -= 1;
            self.current_back_iterator = self.btree.inner[self.current_back_node_idx].inner.iter();

            self.next_back()
        };
    }
}

#[cfg(test)]
mod tests {
    use super::{BTreeSet, Node, DEFAULT_CUTOFF, DEFAULT_INNER_SIZE};

    #[test]
    fn test_insert() {
        let input: Vec<isize> = vec![1, 9, 2, 7, 6, 3, 5, 4, 10, 8];

        let expected_output: Vec<isize> = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10];

        let actual_node = input
            .iter()
            .fold(Node::new(DEFAULT_INNER_SIZE), |mut acc, curr| {
                acc.insert(curr);
                acc
            });

        let actual_output: Vec<isize> = actual_node.inner.into_iter().copied().collect();

        assert_eq!(expected_output, actual_output);
        assert_eq!(*actual_node.max.unwrap(), 10);
    }

    #[test]
    fn test_halve() {
        let mut input: Vec<isize> = vec![];
        for item in 0..DEFAULT_INNER_SIZE {
            input.push(item.clone() as isize);
        }

        let mut former_node = Node::new(DEFAULT_INNER_SIZE);
        input.iter().for_each(|item| {
            former_node.insert(item.clone());
        });
        let latter_node = former_node.halve();

        let expected_former_output: Vec<isize> = input[0..DEFAULT_CUTOFF].to_vec();
        let expected_latter_output: Vec<isize> = input[DEFAULT_CUTOFF..].to_vec();

        let actual_former_output: Vec<isize> = former_node.inner.iter().cloned().collect();
        let actual_latter_output: Vec<isize> = latter_node.inner.iter().cloned().collect();

        assert_eq!(expected_former_output, actual_former_output);
        assert_eq!(expected_latter_output, actual_latter_output);
    }

    #[test]
    fn test_insert_btree() {
        // This will cause the btree to have at least more than one node
        let input = (0..=(DEFAULT_INNER_SIZE * 10)).into_iter().rev();
        let expected_output: Vec<usize> = (0..=(DEFAULT_INNER_SIZE * 10)).collect();

        let btree: BTreeSet<usize> = input
            .into_iter()
            .fold(BTreeSet::new(), |acc, curr| acc.insert_update(curr));
        assert!(btree.inner.len() > 1);

        assert_eq!(btree.len(), expected_output.len());
        for i in 0..=btree.len() {
            assert_eq!(btree.get_index(i), expected_output.get(i).copied());
        }
    }

    #[test]
    fn test_insert_btree_stress() {
        // This will cause the btree to have at least more than one node
        let input = (0..(DEFAULT_INNER_SIZE * 5000)).step_by(1000).rev();
        let expected_output: Vec<usize> = (0..(DEFAULT_INNER_SIZE * 5000)).step_by(1000).collect();

        let btree: BTreeSet<usize> = input
            .into_iter()
            .fold(BTreeSet::new(), |acc, curr| acc.insert_update(curr));
        assert!(btree.inner.len() > 1);

        assert_eq!(btree.len(), expected_output.len());
        for i in 0..=btree.len() {
            assert_eq!(btree.get_index(i), expected_output.get(i).copied());
        }
    }

    #[test]
    fn test_insert_btree_stress2() {
        // This will cause the btree to have at least more than one node
        let input = (0..15_000_000).step_by(100).rev();
        let input_len = input.len();
        let expected_output: Vec<usize> = (0..15_000_000).step_by(100).collect();

        let btree: BTreeSet<usize> = input
            .into_iter()
            .fold(BTreeSet::new(), |acc, curr| acc.insert_update(curr));

        assert_eq!(btree.len(), expected_output.len());
        for i in 0..=btree.len() {
            assert_eq!(btree.get_index(i), expected_output.get(i).copied());
        }

        let input2 = (100_000..100_500).rev();
        let btree = input2
            .into_iter()
            .fold(btree, |acc, curr| acc.insert_update(curr));
        assert!(btree.inner.len() > 1);

        assert_eq!(btree.len(), input_len + 495);

        let input3 = (10_000_000..10_000_500).rev();
        let btree = input3
            .into_iter()
            .fold(btree, |acc, curr| acc.insert_update(curr));
        assert!(btree.inner.len() > 1);

        assert_eq!(btree.len(), input_len + 495 * 2);
    }

    #[test]
    fn test_insert_duplicates() {
        let input: Vec<usize> = (0..(DEFAULT_INNER_SIZE + 1))
            .into_iter()
            .rev()
            .cycle()
            .take(DEFAULT_INNER_SIZE * 3)
            .collect();
        let expected_output: Vec<usize> = (0..(DEFAULT_INNER_SIZE + 1)).collect();

        let btree: BTreeSet<usize> = input
            .into_iter()
            .fold(BTreeSet::new(), |acc, curr| acc.insert_update(curr));
        assert!(btree.inner.len() > 1);

        let mut actual_output: Vec<usize> = vec![];
        for i in 0..btree.len() {
            actual_output.push(btree.get_index(i).unwrap());
        }
        assert_eq!(expected_output.len(), actual_output.len());
        assert_eq!(expected_output, actual_output);
    }

    #[test]
    fn test_remove() {
        let input = 0..(DEFAULT_INNER_SIZE + 1);

        let btree: BTreeSet<usize> = input
            .clone()
            .fold(BTreeSet::new(), |acc, curr| acc.insert_update(curr.clone()));

        let new_tree = input.fold(btree, |acc, item| acc.remove_update(item));

        assert_eq!(new_tree.len(), 0);
    }

    #[test]
    fn test_get_contains_lower_bound() {
        let input = (0..(DEFAULT_INNER_SIZE + 1)).into_iter().rev();
        let expected_output: Vec<usize> = (0..(DEFAULT_INNER_SIZE + 1)).collect();

        let btree: BTreeSet<usize> =
            input.fold(BTreeSet::new(), |acc, curr| acc.insert_update(curr.clone()));

        expected_output.into_iter().for_each(|item| {
            assert_eq!(btree.get_index(item).unwrap(), item);
            assert_eq!(
                btree.get_index(item).unwrap(),
                btree.lower_bound(item).unwrap()
            );
            assert!(btree.contains(item));
        });
    }
}
