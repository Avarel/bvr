use super::viewer::Instance;
use crate::direction::Direction;

#[derive(Clone, Copy)]
pub enum MultiplexerMode {
    Panes,
    Tabs,
}

impl MultiplexerMode {
    pub fn swap(self) -> Self {
        match self {
            Self::Panes => Self::Tabs,
            Self::Tabs => Self::Panes,
        }
    }
}

pub struct MultiplexerApp {
    views: Vec<Instance>,
    mode: MultiplexerMode,
    active: usize,
}

impl MultiplexerApp {
    pub fn new() -> Self {
        Self {
            views: Vec::new(),
            mode: MultiplexerMode::Tabs,
            active: 0,
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.views.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.views.is_empty()
    }

    #[inline]
    pub fn push_viewer(&mut self, viewer: Instance) {
        self.views.push(viewer);
    }

    pub fn close_active_viewer(&mut self) {
        debug_assert!(self.active < self.views.len());
        self.views.remove(self.active);
        self.active = self.active.min(self.views.len().saturating_sub(1));
    }

    pub fn viewer_mut(&mut self, idx: usize) -> &mut Instance {
        &mut self.views[idx]
    }

    pub fn viewers_mut(&mut self) -> &mut Vec<Instance> {
        &mut self.views
    }

    pub fn active(&self) -> usize {
        self.active
    }

    pub fn move_active(&mut self, direction: Direction) {
        self.move_active_index(match direction {
            Direction::Back => self.active.saturating_sub(1),
            Direction::Next => self.active.saturating_add(1),
        })
    }

    pub fn move_active_index(&mut self, index: usize) {
        self.active = index.min(self.views.len().saturating_sub(1));
    }

    pub fn active_viewer_mut(&mut self) -> Option<&mut Instance> {
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

    pub fn set_mode(&mut self, mode: MultiplexerMode) {
        self.mode = mode;
    }
}
