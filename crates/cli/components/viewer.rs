use crate::direction::Direction;

use super::{
    cursor::{Cursor, CursorState, SelectionOrigin},
    filters::Filterer,
    viewport::Viewport,
};
use bitflags::bitflags;
use bvr_core::{SegBuffer, SegStr};
use ratatui::style::Color;
use regex::bytes::Regex;

pub struct Instance {
    name: String,
    buf: SegBuffer,
    viewport: Viewport,
    cursor: CursorState,
    pub filterer: Filterer,
}

bitflags! {
    pub struct LineType: u8 {
        const None = 0;
        const Origin = 1 << 0;
        const Within = 1 << 1;
        const Bookmarked = 1 << 2;
    }
}

pub struct LineData {
    pub line_number: usize,
    pub data: SegStr,
    pub start: usize,
    pub color: Color,
    pub ty: LineType,
}

impl Instance {
    pub fn new(name: String, buf: SegBuffer) -> Self {
        Self {
            name,
            buf,
            cursor: CursorState::new(),
            viewport: Viewport::new(),
            filterer: Filterer::new(),
        }
    }

    pub fn file(&self) -> &SegBuffer {
        &self.buf
    }

    pub fn viewport(&self) -> &Viewport {
        &self.viewport
    }

    pub fn viewport_mut(&mut self) -> &mut Viewport {
        &mut self.viewport
    }

    pub fn visible_line_count(&self) -> usize {
        if self.filterer.filters.all().is_enabled() {
            self.buf.line_count()
        } else {
            self.filterer.composite.len()
        }
    }

    pub fn update_and_view(
        &mut self,
        viewport_height: usize,
        viewport_width: usize,
    ) -> Vec<LineData> {
        self.viewport.fit_view(viewport_height, viewport_width);
        self.viewport.update_end(self.visible_line_count());

        let filters = self.filterer.filters.iter_active().collect::<Vec<_>>();

        let mut lines = Vec::with_capacity(self.viewport.line_range().len());
        for index in self.viewport.line_range() {
            let line_number = if self.filterer.filters.all().is_enabled() {
                index
            } else if let Some(line_number) = self.filterer.composite.get(index) {
                line_number
            } else {
                break;
            };

            let Some(data) = self.buf.get_line(line_number) else {
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
                start: self.viewport.left(),
                color,
                ty: match self.cursor.state {
                    Cursor::Singleton(i) => {
                        if index == i {
                            LineType::Origin
                        } else {
                            LineType::None
                        }
                    }
                    Cursor::Selection(start, end, origin) => {
                        if !(start..=end).contains(&index) {
                            LineType::None
                        } else if index == start && matches!(origin, SelectionOrigin::Left)
                            || index == end && matches!(origin, SelectionOrigin::Right)
                        {
                            LineType::Origin
                        } else {
                            LineType::Within
                        }
                    }
                } | if bookmarked {
                    LineType::Bookmarked
                } else {
                    LineType::None
                },
            });
        }
        lines
    }

    fn line_at_view_index(&mut self, index: usize) -> usize {
        if self.filterer.filters.all().is_enabled() {
            index
        } else {
            self.filterer.composite.get(index).unwrap()
        }
    }

    pub fn filter_search(&mut self, regex: Regex) {
        self.filterer.filter_search(&self.buf, regex);
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn move_selected_into_view(&mut self) {
        let current = match self.cursor.state {
            Cursor::Singleton(i)
            | Cursor::Selection(i, _, SelectionOrigin::Left)
            | Cursor::Selection(_, i, SelectionOrigin::Right) => i,
        };
        if current < self.viewport.top() {
            self.cursor.state = Cursor::Singleton(self.viewport.top());
        } else if current >= self.viewport.bottom() {
            self.cursor.state = Cursor::Singleton(self.viewport.bottom().saturating_sub(1));
        }
    }

    pub fn move_select(&mut self, dir: Direction, select: bool, delta: usize) {
        match dir {
            Direction::Back => self.cursor.back(select, |i| i.saturating_sub(delta)),
            Direction::Next => self.cursor.forward(select, |i| i.saturating_add(delta)),
        }
        self.cursor
            .clamp(self.visible_line_count().saturating_sub(1));
        let i = match self.cursor.state {
            Cursor::Singleton(i)
            | Cursor::Selection(i, _, SelectionOrigin::Left)
            | Cursor::Selection(_, i, SelectionOrigin::Right) => i,
        };
        self.viewport.jump_to(i);
    }

    pub fn toggle_select_bookmarks(&mut self) {
        match self.cursor.state {
            Cursor::Singleton(i) => {
                let line_number = self.line_at_view_index(i);
                self.filterer.filters.bookmarks_mut().toggle(line_number);
            }
            Cursor::Selection(start, end, _) => {
                for i in start..=end {
                    let line_number = self.line_at_view_index(i);
                    self.filterer.filters.bookmarks_mut().toggle(line_number);
                }
            }
        }
        self.cursor
            .clamp(self.visible_line_count().saturating_sub(1));
        self.viewport.update_end(self.visible_line_count());
    }
}
