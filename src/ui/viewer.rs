use std::ops::Range;

use bvr::file::{shard::ShardStr, ShardedFile as RawShardedFile};

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

    pub fn move_view_down(&mut self, delta: usize) {
        self.top = self
            .top
            .saturating_add(delta)
            .min(self.max_height.saturating_sub(1))
    }

    pub fn move_view_up(&mut self, delta: usize) {
        self.top = self.top.saturating_sub(delta)
    }

    pub fn move_select_down(&mut self, delta: usize) {
        self.current = self
            .current
            .saturating_add(delta)
            .min(self.max_height.saturating_sub(1));
        self.jump_to_current()
    }

    pub fn move_select_up(&mut self, delta: usize) {
        self.current = self.current.saturating_sub(delta);
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
            .map(|line_number| {
                (line_number, self.file.get_line(line_number).unwrap())
            })
            .collect()
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

#[derive(Clone, Copy)]
pub enum MultiplexerMode {
    Windows,
    Tabs,
}

impl MultiplexerMode {
    fn swap(self) -> Self {
        match self {
            Self::Windows => Self::Tabs,
            Self::Tabs => Self::Windows,
        }
    }
}

pub struct Multiplexer {
    views: Vec<Viewer>,
    mode: MultiplexerMode,
    active: usize,
}

impl Multiplexer {
    pub fn new() -> Self {
        Self {
            views: Vec::new(),
            mode: MultiplexerMode::Tabs,
            active: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.views.len()
    }

    pub fn is_empty(&self) -> bool {
        self.views.is_empty()
    }

    pub fn push_viewer(&mut self, viewer: Viewer) {
        self.views.push(viewer);
    }

    pub fn close_active_viewer(&mut self) {
        debug_assert!(self.active < self.views.len());
        self.views.remove(self.active);
        self.active = self.active.min(self.views.len().saturating_sub(1));
    }

    pub fn viewer_mut(&mut self, idx: usize) -> &mut Viewer {
        &mut self.views[idx]
    }

    pub fn viewers_mut(&mut self) -> &mut Vec<Viewer> {
        &mut self.views
    }

    pub fn active(&self) -> usize {
        self.active
    }

    pub fn move_active_left(&mut self) {
        self.active = self.active.saturating_sub(1);
    }

    pub fn move_active_right(&mut self) {
        self.active = self.active.saturating_add(1).min(self.views.len().saturating_sub(1));
    }

    pub fn active_viewer_mut(&mut self) -> Option<&mut Viewer> {
        debug_assert!(self.is_empty() || self.active < self.views.len());
        if !self.views.is_empty() {
            Some(self.viewer_mut(self.active))
        } else {
            None
        }
    }

    pub fn mode(&self) -> MultiplexerMode {
        self.mode
    }

    pub(crate) fn swap_mode(&mut self) {
        self.mode = self.mode.swap();
    }
}