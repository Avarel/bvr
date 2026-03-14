use std::sync::{Arc, Mutex};

use crate::cowvec::{CowVec, CowVecWriter};


/// An exclusive writer to a `SplitCowVec<T>`.
///
/// This writer manages multiple `CowVec<T>` segments, creating a new segment
/// every `elements_per_segment` elements. The writer and reader can coexist,
/// allowing concurrent reads while writing.
pub struct SplitCowVecWriter<T> {
    elements_per_segment: usize,
    segments: Arc<Mutex<Vec<Arc<CowVec<T>>>>>,
    current: Option<(Arc<CowVec<T>>, CowVecWriter<T>)>,
}

impl<T> SplitCowVecWriter<T>
where
    T: Copy,
{
    /// Appends an element to the back of the split vector.
    ///
    /// This operation is O(1) amortized. When the current segment reaches
    /// `elements_per_segment`, a new segment is created.
    pub fn push(&mut self, elem: T) {
        let writer = if let Some((_, writer)) = self.current.as_mut() {
            if writer.len() >= self.elements_per_segment {
                Self::create_new_segment(self)
            } else {
                writer
            }
        } else {
            Self::create_new_segment(self)
        };

        writer.push(elem);
    }

    /// Creates a new segment and switches to it.
    fn create_new_segment(&mut self) -> &mut CowVecWriter<T> {
        // Save the current segment if it exists
        if let Some((segment, _)) = self.current.take() {
            self.segments.lock().unwrap().push(segment);
        }

        // Create a new CowVec
        &mut self.current.insert(CowVec::new()).1
    }

    /// Returns the total number of elements written so far.
    pub fn len(&self) -> usize {
        self.segments.lock().unwrap().iter().map(|seg| seg.len()).sum()
    }

    /// Returns true if no elements have been written.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the number of segments (completed + current).
    pub fn segment_count(&self) -> usize {
        self.segments.lock().unwrap().len()
    }
}

impl<T> Drop for SplitCowVecWriter<T> {
    fn drop(&mut self) {
        // Finalize the current segment if it exists
        if let Some((segment, _)) = self.current.take() {
            self.segments.lock().unwrap().push(segment);
        }
    }
}

/// A read-only view of a split copy-on-write vector.
///
/// This is composed of multiple `CowVec<T>` segments, each containing up to
/// `elements_per_segment` elements. This can coexist with a `SplitCowVecWriter`,
/// allowing concurrent reads while writing.
pub struct SplitCowVec<T> {
    segments: Arc<Mutex<Vec<Arc<CowVec<T>>>>>,
}

impl<T> SplitCowVec<T> {
    /// Constructs a new, empty `SplitCowVec<T>` with a write handle.
    ///
    /// The vector will not allocate until elements are pushed onto it.
    ///
    /// # Arguments
    /// * `elements_per_segment` - Number of elements per segment before creating a new CowVec
    pub fn new(elements_per_segment: usize) -> (Self, SplitCowVecWriter<T>) {
        let segments = Arc::new(Mutex::new(Vec::new()));

        let writer = SplitCowVecWriter {
            elements_per_segment,
            segments: segments.clone(),
            current: None,
        };

        (
            SplitCowVec {
                segments: segments,
            },
            writer,
        )
    }

    /// Constructs a new, empty `SplitCowVec<T>` with default configuration (1024 elements per segment).
    pub fn with_default_config() -> (Self, SplitCowVecWriter<T>) {
        Self::new(1024)
    }

    /// Returns the total number of elements across all segments.
    pub fn len(&self) -> usize {
        self.segments.lock().unwrap().iter().map(|seg| seg.len()).sum()
    }

    /// Returns the number of segments.
    pub fn segment_count(&self) -> usize {
        self.segments.lock().unwrap().len()
    }

    /// Returns true if the split vector contains no elements.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Takes an atomic snapshot of all segments at the current point in time.
    ///
    /// This snapshot pins all internal buffers, ensuring a consistent view
    /// across all segments even if writes occur after the snapshot is taken.
    pub fn snapshot(&self) -> SplitCowVecSnapshot<T> {
        let snapshots = self.segments.lock().unwrap().iter().map(|seg| seg.snapshot()).collect();

        SplitCowVecSnapshot { snapshots }
    }
}

impl<T> Clone for SplitCowVec<T> {
    fn clone(&self) -> Self {
        Self {
            segments: self.segments.clone(),
        }
    }
}

impl<T> std::fmt::Debug for SplitCowVec<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SplitCowVec[..]")
    }
}

/// A snapshot of a `SplitCowVec<T>` at a point in time.
pub struct SplitCowVecSnapshot<T> {
    snapshots: Vec<crate::cowvec::CowVecSnapshot<T>>,
}

impl<T> SplitCowVecSnapshot<T> {
    /// Returns the number of segments in this snapshot.
    pub fn segment_count(&self) -> usize {
        self.snapshots.len()
    }

    /// Returns a snapshot of the segment at the given index, or `None` if out of bounds.
    pub fn get_segment(&self, index: usize) -> Option<&crate::cowvec::CowVecSnapshot<T>> {
        self.snapshots.get(index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_cowvec_basic() {
        let (vec, mut writer) = SplitCowVec::new(5);

        for i in 0..12 {
            writer.push(i);
        }

        drop(writer);

        assert_eq!(vec.len(), 12);
        assert_eq!(vec.segment_count(), 3); // 5 + 5 + 2
    }

    #[test]
    fn test_split_cowvec_single_segment() {
        let (vec, mut writer) = SplitCowVec::new(100);

        for i in 0..10 {
            writer.push(i);
        }

        drop(writer);

        assert_eq!(vec.len(), 10);
        assert_eq!(vec.segment_count(), 1);
    }

    #[test]
    fn test_split_cowvec_empty() {
        let (_vec, _writer) = SplitCowVec::<i32>::with_default_config();
        // Writer is dropped, so segments are finalized
        assert!(_vec.is_empty());
        assert_eq!(_vec.segment_count(), 0);
    }

    #[test]
    fn test_split_cowvec_snapshot() {
        let (vec, mut writer) = SplitCowVec::new(3);

        for i in 0..7 {
            writer.push(i);
        }

        drop(writer);

        let snapshot = vec.snapshot();
        assert_eq!(snapshot.segment_count(), 3);
    }
}
