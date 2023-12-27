use super::{
    cursor::{Cursor, CursorState, SelectionOrigin},
    filters::Filterer,
    viewport::Viewport,
};
use crate::{app::ViewDelta, direction::Direction};
use bitflags::bitflags;
use bvr_core::Result;
use bvr_core::{SegBuffer, SegStr};
use ratatui::style::Color;
use regex::bytes::Regex;
use std::{collections::BTreeMap, fs::File};

pub struct Instance {
    name: String,
    buf: SegBuffer,
    viewport: Viewport,
    cursor: CursorState,
    pub filterer: Filterer,
    // context: BTreeMap<usize, usize>,
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
            // context: BTreeMap::new(),
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
        if let Some(composite) = self.filterer.composite.as_ref() {
            composite.len()
        } else {
            self.buf.line_count()
        }
    }

    fn line_at_view_index(&self, index: usize) -> Option<usize> {
        if let Some(composite) = self.filterer.composite.as_ref() {
            composite.get(index)
        } else {
            Some(index)
        }
    }

    // pub fn translate_context_to_realspace(&self) -> usize {
    //     let mut top = self.viewport.top();

    //     for (&k, &v) in self.context.iter() {
    //         if k < top {
    //             top = top.saturating_sub(v);
    //         } else {
    //             break;
    //         }
    //     }

    //     top
    // }

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
            let Some(line_number) = self.line_at_view_index(index) else {
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
                ty: match self.cursor.state() {
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

    pub fn filter_search(&mut self, regex: Regex) {
        self.filterer.filter_search(&self.buf, regex);
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn move_selected_into_view(&mut self) {
        let current = match self.cursor.state() {
            Cursor::Singleton(i)
            | Cursor::Selection(i, _, SelectionOrigin::Left)
            | Cursor::Selection(_, i, SelectionOrigin::Right) => i,
        };
        if current < self.viewport.top() {
            self.cursor.place(self.viewport.top());
        } else if current >= self.viewport.bottom() {
            self.cursor.place(self.viewport.bottom().saturating_sub(1));
        }
    }

    pub fn move_select(&mut self, dir: Direction, select: bool, delta: ViewDelta) {
        let ndelta = match delta {
            ViewDelta::Number(n) => usize::from(n),
            ViewDelta::Page => self.viewport.height(),
            ViewDelta::HalfPage => self.viewport.height().div_ceil(2),
            ViewDelta::Boundary => usize::MAX,
            ViewDelta::Match => 0,
        };

        match dir {
            Direction::Back => self.cursor.back(select, |i| {
                let delta = match delta {
                    ViewDelta::Match => return self.filterer.compute_jump(i, dir).unwrap_or(i),
                    _ => ndelta,
                };
                i.saturating_sub(delta)
            }),
            Direction::Next => self.cursor.forward(select, |i| {
                let delta = match delta {
                    ViewDelta::Match => return self.filterer.compute_jump(i, dir).unwrap_or(i),
                    _ => ndelta,
                };
                i.saturating_add(delta)
            }),
        }
        self.cursor
            .clamp(self.visible_line_count().saturating_sub(1));
        let i = match self.cursor.state() {
            Cursor::Singleton(i)
            | Cursor::Selection(i, _, SelectionOrigin::Left)
            | Cursor::Selection(_, i, SelectionOrigin::Right) => i,
        };
        self.viewport.jump_to(i);
    }

    pub fn toggle_select_bookmarks(&mut self) {
        match self.cursor.state() {
            Cursor::Singleton(i) => {
                let line_number = self.line_at_view_index(i).unwrap();
                self.filterer.filters.bookmarks_mut().toggle(line_number);
            }
            Cursor::Selection(start, end, _) => {
                for i in (start..=end).rev() {
                    let line_number = self.line_at_view_index(i).unwrap();
                    self.filterer.filters.bookmarks_mut().toggle(line_number);
                }
            }
        }
        self.cursor
            .clamp(self.visible_line_count().saturating_sub(1));
        self.viewport.update_end(self.visible_line_count());
    }

    pub fn export_file(&mut self, file: File) -> Result<()> {
        self.buf
            .write_file(file, self.filterer.composite.as_ref().unwrap().clone())
    }
}
