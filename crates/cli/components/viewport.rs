use std::ops::Range;

use crate::direction::Direction;

pub struct Viewport {
    // End of the view
    vend: usize,
    /// Top of the view
    top: usize,
    /// Left of the view
    left: usize,
    /// Visible height
    height: usize,
    width: usize,
    /// True if the view should follow the output
    follow_output: bool,
}

impl Viewport {
    #[inline]
    pub const fn new() -> Self {
        Self {
            vend: 0,
            top: 0,
            left: 0,
            height: 0,
            width: 0,
            follow_output: false,
        }
    }

    #[inline(always)]
    pub fn height(&self) -> usize {
        self.height
    }

    #[inline(always)]
    pub fn width(&self) -> usize {
        self.width
    }

    pub fn fit_view(&mut self, height: usize, width: usize) {
        self.height = height;
        self.width = width;
        self.fixup();
    }

    #[inline]
    pub fn bottom(&self) -> usize {
        self.top + self.height
    }

    fn fixup(&mut self) {
        if self.top >= self.vend {
            self.top = self.vend.saturating_sub(1);
        }
        if self.height > self.vend {
            self.height = self.vend;
        }
        if self.follow_output {
            self.top = self.vend.saturating_sub(self.height);
        }
    }

    pub fn jump_to(&mut self, index: usize) {
        if !(self.top..self.bottom()).contains(&index) {
            // height remains unchanged
            if self.top.abs_diff(index) < self.bottom().abs_diff(index) {
                // bring the top to current
                self.top = index;
            } else {
                // bring the bottom to current
                self.top = index.saturating_sub(self.height).saturating_add(1);
            }
        }
    }

    pub fn pan_vertical(&mut self, direction: Direction, delta: usize) {
        self.follow_output = false;
        self.top = match direction {
            Direction::Back => self.top.saturating_sub(delta),
            Direction::Next => self
                .top
                .saturating_add(delta)
                .min(self.vend.saturating_sub(1)),
        }
    }

    pub fn pan_horizontal(&mut self, direction: Direction, delta: usize) {
        self.left = match direction {
            Direction::Back => self.left.saturating_sub(delta),
            Direction::Next => self.left.saturating_add(delta),
        }
    }

    #[inline(always)]
    pub fn follow_output(&mut self) {
        self.follow_output = true;
    }

    pub fn update_end(&mut self, max_height: usize) {
        self.vend = max_height;
        self.fixup();
    }

    #[inline]
    pub fn line_range(&self) -> Range<usize> {
        self.top..self.bottom().min(self.vend)
    }

    #[inline(always)]
    pub fn left(&self) -> usize {
        self.left
    }

    #[inline(always)]
    pub(crate) fn top(&self) -> usize {
        self.top
    }
}
