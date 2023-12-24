use crate::direction::VDirection;
use std::ops::Range;

pub struct Viewport {
    max_height: usize,
    top: usize,
    height: usize,
    current: usize,
    follow_output: bool,
}

impl Viewport {
    pub const fn new() -> Self {
        Self {
            max_height: 0,
            top: 0,
            height: 0,
            current: 0,
            follow_output: false,
        }
    }

    pub fn height(&self) -> usize {
        self.height
    }

    pub fn fit_view(&mut self, height: usize) {
        self.height = height;
        self.fixup();
    }

    pub(crate) fn bottom(&self) -> usize {
        self.top + self.height
    }

    pub(crate) fn fixup(&mut self) {
        if self.top >= self.max_height {
            self.top = self.max_height.saturating_sub(1);
        }
        if self.height > self.max_height {
            self.height = self.max_height;
        }
        if self.current >= self.max_height {
            self.current = self.max_height.saturating_sub(1);
        }
        if self.follow_output {
            self.top = self.max_height.saturating_sub(self.height);
        }
    }

    pub fn move_select_within_view(&mut self) {
        if self.current < self.top {
            self.current = self.top;
        } else if self.current >= self.bottom() {
            self.current = self.bottom().saturating_sub(1);
        }
    }

    pub(crate) fn jump_to_current(&mut self) {
        if !(self.top..self.bottom()).contains(&self.current) {
            // height remains unchanged
            if self.top.abs_diff(self.current) < self.bottom().abs_diff(self.current) {
                // bring the top to current
                self.top = self.current;
            } else {
                // bring the bottom to current
                self.top = self.current.saturating_sub(self.height).saturating_add(1);
            }
        }
    }

    pub fn pan_view(&mut self, direction: VDirection, delta: usize) {
        self.follow_output = false;
        self.top = match direction {
            VDirection::Up => self.top.saturating_sub(delta),
            VDirection::Down => self
                .top
                .saturating_add(delta)
                .min(self.max_height.saturating_sub(1)),
        }
    }

    pub fn follow_output(&mut self) {
        self.follow_output = true;
    }

    pub fn update_max_height(&mut self, max_height: usize) {
        self.max_height = max_height;
        self.fixup();
    }

    pub fn move_select(&mut self, direction: VDirection, delta: usize) {
        self.current = match direction {
            VDirection::Up => self.current.saturating_sub(delta),
            VDirection::Down => self
                .current
                .saturating_add(delta)
                .min(self.max_height.saturating_sub(1)),
        };
        self.jump_to_current()
    }

    pub fn line_range(&self) -> Range<usize> {
        self.top..self.bottom().min(self.max_height)
    }

    pub fn current(&self) -> usize {
        self.current
    }
}
