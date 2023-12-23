use crate::cowvec::CowVec;

pub mod inflight;

#[derive(Clone)]
pub struct IncompleteComposite {
    inner: Composite,
}

impl IncompleteComposite {
    /// Create a new [IncompleteComposite].
    pub fn new() -> Self {
        Self {
            inner: Composite::empty(),
        }
    }

    pub fn add_line(&mut self, line_number: usize) {
        if self.inner.lines.last() == Some(&line_number) {
            return;
        } else if let Some(last) = self.inner.lines.last() {
            assert!(line_number > *last);
        }
        self.inner.lines.push(line_number)
    }

    #[must_use]
    pub fn finish(self) -> Composite {
        self.inner
    }
}

impl Default for IncompleteComposite {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone)]
pub struct Composite {
    lines: CowVec<usize>,
}

impl Composite {
    pub fn empty() -> Self {
        Self {
            lines: CowVec::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn get(&self, index: usize) -> Option<usize> {
        self.lines.get(index).copied()
    }
}