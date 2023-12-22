use bvr_core::cowvec::CowVec;

pub mod inflight;

#[derive(Clone)]
pub struct IncompleteComposite {
    inner: CompleteComposite,
}

impl IncompleteComposite {
    /// Create a new [IncompleteComposite].
    pub fn new() -> Self {
        Self {
            inner: CompleteComposite::empty(),
        }
    }

    pub fn add_line(&mut self, line_number: usize) {
        if self.inner.lines.last() == Some(&line_number) {
            return;
        }
        self.inner.lines.push(line_number)
    }

    #[must_use]
    pub fn finish(self) -> CompleteComposite {
        self.inner
    }
}

impl Default for IncompleteComposite {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone)]
pub struct CompleteComposite {
    lines: CowVec<usize>,
}

impl CompleteComposite {
    pub fn empty() -> Self {
        Self {
            lines: CowVec::new(),
        }
    }
}