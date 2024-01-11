use crate::direction::Direction;

#[derive(Clone, Copy)]
pub struct Viewport {
    /// Top of the view
    top: usize,
    /// Left of the view
    left: usize,
    /// Visible height
    height: usize,
    /// Visible width
    width: usize,
}

impl Viewport {
    #[inline]
    pub const fn new() -> Self {
        Self {
            top: 0,
            left: 0,
            height: 0,
            width: 0,
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
    }
    
    #[inline(always)]
    pub fn left(&self) -> usize {
        self.left
    }

    #[inline(always)]
    pub(crate) fn top(&self) -> usize {
        self.top
    }

    #[inline(always)]
    pub fn right(&self) -> usize {
        self.left + self.width
    }

    #[inline(always)]
    pub fn bottom(&self) -> usize {
        self.top + self.height
    }

    pub fn clamp(&mut self, end_index: usize) {
        if self.top >= end_index {
            self.top = end_index.saturating_sub(1);
        }
        if self.height > end_index {
            self.height = end_index;
        }
    }

    pub fn top_to(&mut self, index: usize) {
        self.top = index;
    }

    pub fn jump_vertically_to(&mut self, index: usize) {
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

    pub fn jump_horizontally_to(&mut self, index: usize) {
        if !(self.left..self.right()).contains(&index) {
            // width remains unchanged
            if self.left.abs_diff(index) < self.right().abs_diff(index) {
                // bring the left to current
                self.left = index;
            } else {
                // bring the right to current
                self.left = index.saturating_sub(self.width).saturating_add(1);
            }
        }
    }

    pub fn pan_vertical(&mut self, direction: Direction, delta: usize) {
        self.top = match direction {
            Direction::Back => self.top.saturating_sub(delta),
            Direction::Next => self.top.saturating_add(delta),
        }
    }

    pub fn pan_horizontal(&mut self, direction: Direction, delta: usize) {
        self.left = match direction {
            Direction::Back => self.left.saturating_sub(delta),
            Direction::Next => self.left.saturating_add(delta),
        }
    }

}
