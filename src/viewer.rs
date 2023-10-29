use std::ops::Range;

pub struct Viewport {
    max_height: usize,
    top: usize,
    height: usize,
    current: usize,
}

impl Viewport {
    pub(super) fn new() -> Self {
        Self {
            max_height: 100,
            top: 0,
            height: 0,
            current: 0,
        }
    }

    fn bottom(&self) -> usize {
        self.top + self.height
    }

    pub fn fit_view(&mut self, height: usize) {
        self.height = height;
    }

    fn jump_to_current(&mut self) {
        if !(self.top..self.bottom()).contains(&self.current) {
            // height remains unchanged
            if self.top.abs_diff(self.current) < self.bottom().abs_diff(self.current) {
                // bring the top to current
                self.top = self.current;
            } else {
                // bring the bottom to current
                self.top = self.current.saturating_sub(self.height);
            }
        }
    }

    pub fn move_down(&mut self) {
        self.top = self.top.saturating_add(1).min(self.max_height.saturating_sub(1))
    }

    pub fn move_up(&mut self) {
        self.top = self.top.saturating_sub(1)
    }

    pub fn line_range(&self) -> Range<usize> {
        self.top..self.bottom()
    }

    pub fn current(&self) -> usize {
        self.current
    }

    pub fn max_height(&self) -> usize {
        self.max_height
    }
}

pub(super) struct Viewer {
    viewport: Viewport,
}

impl Viewer {
    pub(super) fn new() -> Self {
        Self {
            viewport: Viewport::new(),
        }
    }

    pub fn viewport_mut(&mut self) -> &mut Viewport {
        &mut self.viewport
    }

    pub fn viewport(&self) -> &Viewport {
        &self.viewport
    }
}
