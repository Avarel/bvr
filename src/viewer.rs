use std::ops::Range;

use dltwf::file::{shard::ShardStr, ShardedFile};

pub struct Viewport {
    max_height: usize,
    top: usize,
    height: usize,
    current: usize,
}

impl Viewport {
    pub(super) fn new(max_height: usize) -> Self {
        Self {
            max_height,
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
        self.top = self
            .top
            .saturating_add(1)
            .min(self.max_height.saturating_sub(1))
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
    file: ShardedFile,
}

impl Viewer {
    pub(super) fn new(file: ShardedFile) -> Self {
        Self {
            viewport: Viewport::new(file.line_count() + 1),
            file,
        }
    }

    pub fn viewport_mut(&mut self) -> &mut Viewport {
        &mut self.viewport
    }

    pub fn viewport(&self) -> &Viewport {
        &self.viewport
    }

    pub fn view(&self) -> Vec<Option<(usize, ShardStr)>> {
        self.viewport.line_range().map(|line_number| {
            if line_number < self.viewport.max_height() {
                Some((line_number, self.file.get_line(line_number).unwrap()))
            } else {
                None
            }
        }).collect()
    }
}
