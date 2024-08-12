use super::instance::Instance;
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
    instances: Vec<Instance>,
    mode: MultiplexerMode,
    active: usize,
}

impl MultiplexerApp {
    pub fn new() -> Self {
        Self {
            instances: Vec::new(),
            mode: MultiplexerMode::Tabs,
            active: 0,
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.instances.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.instances.is_empty()
    }

    #[inline]
    pub fn push(&mut self, instance: Instance) {
        self.instances.push(instance);
    }

    pub fn instances_mut(&mut self) -> &mut Vec<Instance> {
        &mut self.instances
    }

    pub fn active_index(&self) -> usize {
        self.active
    }

    pub fn move_active_index(&mut self, index: usize) {
        self.active = index.min(self.instances.len().saturating_sub(1));
    }

    pub fn move_active(&mut self, direction: Direction) {
        self.move_active_index(match direction {
            Direction::Back => self.active.saturating_sub(1),
            Direction::Next => self.active.saturating_add(1),
        })
    }

    pub fn close_active(&mut self) {
        debug_assert!(self.active < self.instances.len());
        self.instances.remove(self.active);
        self.active = self.active.min(self.instances.len().saturating_sub(1));
    }

    pub fn active_mut(&mut self) -> Option<&mut Instance> {
        debug_assert!(self.is_empty() || self.active < self.instances.len());
        self.instances.get_mut(self.active)
    }

    pub fn demux_mut<F>(&mut self, linked: bool, f: F)
    where
        F: FnMut(&mut Instance),
    {
        if linked {
            self.instances.iter_mut().for_each(f)
        } else {
            self.active_action(f)
        }
    }

    pub fn active_action<F>(&mut self, mut f: F)
    where
        F: FnMut(&mut Instance),
    {
        if let Some(instance) = self.active_mut() {
            f(instance);
        }
    }

    pub fn mode(&self) -> MultiplexerMode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: MultiplexerMode) {
        self.mode = mode;
    }

    pub fn clear(&mut self) {
        self.instances.clear();
        self.active = 0;
    }
}
