use bvr_core::{matches::BufferMatches, SegStr};
use ratatui::style::Color;
use regex::bytes::Regex;
use std::ops::Range;

use crate::direction::VDirection;

use super::filters::Filterer;

pub type Buffer = bvr_core::SegBuffer<bvr_core::InflightIndex>;

pub struct Viewport {
    pub max_height: usize,
    pub top: usize,
    pub height: usize,
    pub current: usize,
}

impl Viewport {
    pub const fn new() -> Self {
        Self {
            max_height: 0,
            top: 0,
            height: 0,
            current: 0,
        }
    }

    pub fn height(&self) -> usize {
        self.height
    }

    fn bottom(&self) -> usize {
        self.top + self.height
    }

    fn fixup(&mut self) {
        if self.top >= self.max_height {
            self.top = self.max_height.saturating_sub(1);
        }
        if self.height > self.max_height {
            self.height = self.max_height;
        }
        if self.current >= self.max_height {
            self.current = self.max_height.saturating_sub(1);
        }
    }

    pub fn move_select_within_view(&mut self) {
        if self.current < self.top {
            self.current = self.top;
        } else if self.current >= self.bottom() {
            self.current = self.bottom().saturating_sub(1);
        }
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

pub struct Instance {
    name: String,
    file: Buffer,
    viewport: Viewport,
    pub filterer: Filterer,
}

pub struct LineData {
    pub line_number: usize,
    pub data: SegStr,
    pub color: Color,
    pub bookmarked: bool,
    pub selected: bool,
}

impl Instance {
    pub fn new(name: String, file: Buffer) -> Self {
        Self {
            name,
            file,
            viewport: Viewport::new(),
            filterer: Filterer::new(),
        }
    }

    pub fn file(&self) -> &Buffer {
        &self.file
    }

    pub fn viewport(&self) -> &Viewport {
        &self.viewport
    }

    pub fn viewport_mut(&mut self) -> &mut Viewport {
        &mut self.viewport
    }

    pub fn update_and_view(&mut self, viewport_height: usize) -> Vec<LineData> {
        self.file.try_finalize();
        self.filterer.filters.try_finalize();
        self.filterer.composite.try_finalize();

        self.viewport.height = viewport_height;

        let mut lines = Vec::with_capacity(self.viewport.line_range().len());
        if self.filterer.filters.all().is_enabled() {
            self.viewport.max_height = self.file.line_count();
            self.viewport.fixup();
        } else {
            self.viewport.max_height = self.filterer.composite.len();
            self.viewport.fixup();
        }

        let filters = self.filterer.filters.iter_active().collect::<Vec<_>>();

        for index in self.viewport.line_range() {
            let line_number = if self.filterer.filters.all().is_enabled() {
                index
            } else {
                self.filterer
                    .composite
                    .get(index)
                    .expect("valid index into composite")
            };

            let Some(data) = self.file.get_line(line_number) else {
                break;
            };
            let color = filters
                .iter()
                .rev()
                .find(|filter| filter.has_line(line_number))
                .map(|filter| filter.color)
                .unwrap_or(Color::White);

            let bookmarked = self.filterer.filters.bookmarks().has_line(line_number);

            lines.push(LineData {
                line_number,
                data,
                color,
                bookmarked,
                selected: index == self.viewport.current,
            });
        }
        lines
    }

    pub fn current_selected_file_line(&mut self) -> usize {
        if self.filterer.filters.all().is_enabled() {
            self.viewport.current()
        } else {
            self.filterer
                .composite
                .get(self.viewport.current())
                .unwrap()
        }
    }

    pub fn filter_search(&mut self, regex: Regex) {
        self.filterer.filter_search(&self.file, regex);
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}
