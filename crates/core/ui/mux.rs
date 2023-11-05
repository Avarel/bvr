use crate::common::HDirection;

use super::viewer::Viewer;

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

pub struct MultiplexerApp {
    views: Vec<Viewer>,
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

    pub fn move_active(&mut self, direction: HDirection) {
        self.active = match direction {
            HDirection::Left => self.active.saturating_sub(1),
            HDirection::Right => self
                .active
                .saturating_add(1)
                .min(self.views.len().saturating_sub(1)),
        }
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
