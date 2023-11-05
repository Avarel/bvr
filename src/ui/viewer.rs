use std::ops::Range;

use bvr::file::{shard::ShardStr, ShardedFile as RawShardedFile};

use crate::common::VDirection;

pub struct Viewport {
    max_height: usize,
    top: usize,
    height: usize,
    current: usize,
}

impl Viewport {
    const fn new() -> Self {
        Self {
            max_height: 0,
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
                self.top = self.current.saturating_sub(self.height).saturating_add(1);
            }
        }
    }

    pub fn pan_view(&mut self, direction: VDirection, delta: usize) {
        self.top = match direction {
            VDirection::Up => self.top.saturating_sub(delta),
            VDirection::Down => self
                .top
                .saturating_add(delta)
                .min(self.max_height.saturating_sub(1)),
        }
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

type ShardedFile = RawShardedFile<bvr::index::sync::AsyncIndex>;

pub struct Viewer {
    name: String,
    viewport: Viewport,
    file: ShardedFile,
}

impl Viewer {
    pub fn new(name: String, file: ShardedFile) -> Self {
        Self {
            name,
            viewport: Viewport::new(),
            file,
        }
    }

    pub fn file(&self) -> &ShardedFile {
        &self.file
    }

    pub fn viewport_mut(&mut self) -> &mut Viewport {
        &mut self.viewport
    }

    pub fn update_and_view(&mut self) -> Vec<(usize, ShardStr)> {
        self.file.try_finalize();
        self.viewport.max_height = self.file.line_count();
        self.viewport
            .line_range()
            .map(|line_number| (line_number, self.file.get_line(line_number).unwrap()))
            .collect()
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}
