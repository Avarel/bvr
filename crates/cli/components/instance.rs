use super::{
    cursor::{Cursor, CursorState, SelectionOrigin},
    filters::Compositor,
    viewport::Viewport,
};
use crate::{app::ViewDelta, direction::Direction};
use bitflags::bitflags;
use bvr_core::{LineMatches, Result};
use bvr_core::{SegBuffer, SegStr};
use ratatui::style::Color;
use regex::bytes::Regex;
use std::{collections::VecDeque, fs::File};

bitflags! {
    #[derive(Clone)]
    pub struct LineType: u8 {
        const None = 0;
        const Origin = 1 << 0;
        const OriginStart = 1 << 1;
        const OriginEnd = 1 << 2;
        const Within = 1 << 3;
        const Bookmarked = 1 << 4;
    }
}

#[derive(Clone)]
pub struct LineData<'a> {
    pub line_number: usize,
    pub data: &'a str,
    pub start: usize,
    pub color: Color,
    pub ty: LineType,
}

#[derive(Clone)]
struct CachedLine {
    index: usize,
    line_number: usize,
    data: SegStr,
    color: Color,
    bookmarked: bool,
}

pub struct ViewManager {
    composite: Option<LineMatches>,

    cache: VecDeque<CachedLine>,

    prev_viewport: Viewport,
    curr_viewport: Viewport,

    follow_output: bool,
    end_index: usize,

    need_recoloring: bool,
    pub compositor: Compositor,
}

impl ViewManager {
    fn new() -> Self {
        Self {
            composite: None,
            cache: VecDeque::new(),
            prev_viewport: Viewport::new(),
            curr_viewport: Viewport::new(),
            follow_output: false,
            need_recoloring: false,
            end_index: 0,
            compositor: Compositor::new(),
        }
    }

    pub fn set_follow_output(&mut self, follow_output: bool) {
        self.follow_output = follow_output;
    }

    pub fn viewport(&self) -> &Viewport {
        &self.curr_viewport
    }

    pub fn viewport_mut(&mut self) -> &mut Viewport {
        &mut self.curr_viewport
    }

    fn line_at_view_index(&self, index: usize) -> Option<usize> {
        if let Some(composite) = self.composite() {
            composite.get(index)
        } else if index < self.end_index {
            Some(index)
        } else {
            None
        }
    }

    fn push_front(&mut self, index: usize, buf: &SegBuffer) {
        let Some(line_number) = self.line_at_view_index(index) else {
            return;
        };

        let Some(data) = buf.get_line(line_number) else {
            todo!("push empty line");
        };

        self.cache.push_front(CachedLine {
            index,
            line_number,
            data,
            color: Color::Reset,
            bookmarked: false,
        });
    }

    fn push_back(&mut self, index: usize, buf: &SegBuffer) -> bool {
        let Some(line_number) = self.line_at_view_index(index) else {
            return false;
        };

        let Some(data) = buf.get_line(line_number) else {
            return false;
        };

        self.cache.push_back(CachedLine {
            index,
            line_number,
            data,
            color: Color::Reset,
            bookmarked: false,
        });
        true
    }

    fn compute_view(&mut self, buf: &SegBuffer) -> impl Iterator<Item = &CachedLine> {
        if self.follow_output {
            self.curr_viewport.jump_to(self.end_index.saturating_sub(1));
        }

        let (old_top, new_top) = (self.prev_viewport.top(), self.curr_viewport.top());
        let (old_bot, new_bot) = (self.prev_viewport.bottom(), self.curr_viewport.bottom());

        self.curr_viewport.clamp(self.end_index);

        if new_top > old_bot || new_bot < old_top {
            // No overlap between old and new viewports
            self.cache.clear();
        } else {
            // Overlap between old and new viewports
            // Shift the cache to match the new viewport
            if old_top < new_top {
                for _ in old_top..new_top {
                    self.cache.pop_front();
                }
            } else if old_top > new_top {
                for i in (new_top..old_top).rev() {
                    self.push_front(i, buf);
                }
            }
        }

        // Populate the cache to fill the viewport
        while self.cache.len() < self.curr_viewport.height() {
            if !self.push_back(self.cache.len() + new_top, buf) {
                break;
            }
        }

        self.cache.truncate(self.curr_viewport.height());

        self.prev_viewport = self.curr_viewport.clone();

        self.color_cache();

        self.cache.iter()
    }

    fn color_cache(&mut self) {
        if self.need_recoloring {
            self.reset_color_cache();
            self.need_recoloring = self
                .compositor
                .filters()
                .iter_active()
                .any(|filter| !filter.is_complete());
        }

        let filters = self.compositor.filters().iter_active().collect::<Vec<_>>();

        self.cache
            .iter_mut()
            .filter(|line| line.color == Color::Reset)
            .for_each(|line| {
                line.color = filters
                    .iter()
                    .rev()
                    .find(|filter| filter.has_line(line.line_number))
                    .map(|filter| filter.color())
                    .unwrap_or(Color::White);

                line.bookmarked = self
                    .compositor
                    .filters()
                    .bookmarks()
                    .has_line(line.line_number);
            });
    }

    pub fn reset_color_cache(&mut self) {
        self.need_recoloring = true;
        self.cache
            .iter_mut()
            .for_each(|line| line.color = Color::Reset);
    }

    pub fn invalidate_cache(&mut self) {
        let prev_all = self.composite.is_none();
        let now_all = !self.compositor.needs_composite();

        if prev_all && now_all {
            self.reset_color_cache();
        } else {
            self.cache.clear();
            self.compute_composite();
        }
    }

    fn compute_composite(&mut self) {
        if !self.compositor.needs_composite() {
            self.composite = None;
            return;
        }
        self.composite = Some(self.compositor.create_composite());
    }

    pub fn compute_jump(&self, i: usize, direction: Direction) -> Option<usize> {
        if self.composite.is_some() {
            // The composite is literally all matches
            return match direction {
                Direction::Back => Some(i.saturating_sub(1)),
                Direction::Next => Some(i.saturating_add(1)),
            };
        }
        match direction {
            Direction::Back => self
                .compositor
                .filters()
                .iter_active()
                .filter_map(|filter| filter.nearest_backward(i))
                .filter(|&ln| ln < i)
                .max(),
            Direction::Next => self
                .compositor
                .filters()
                .iter_active()
                .filter_map(|filter| filter.nearest_forward(i))
                .filter(|&ln| ln > i)
                .min(),
        }
    }

    pub fn composite(&self) -> Option<&LineMatches> {
        self.composite.as_ref()
    }
}

pub struct Instance {
    name: String,
    buf: SegBuffer,
    cursor: CursorState,
    pub view: ViewManager,
}

impl Instance {
    pub fn new(name: String, buf: SegBuffer) -> Self {
        Self {
            name,
            buf,
            cursor: CursorState::new(),
            view: ViewManager::new(),
        }
    }

    pub fn file(&self) -> &SegBuffer {
        &self.buf
    }

    pub fn viewport(&self) -> &Viewport {
        self.view.viewport()
    }

    pub fn viewport_mut(&mut self) -> &mut Viewport {
        self.view.viewport_mut()
    }

    pub fn set_follow_output(&mut self, follow_output: bool) {
        self.view.set_follow_output(follow_output);
    }

    pub fn visible_line_count(&self) -> usize {
        if let Some(composite) = self.view.composite() {
            composite.len()
        } else {
            self.buf.line_count()
        }
    }

    fn line_at_view_index(&self, index: usize) -> Option<usize> {
        if let Some(composite) = self.view.composite() {
            composite.get(index)
        } else if index < self.buf.line_count() {
            Some(index)
        } else {
            None
        }
    }

    pub fn nearest_index(&self, line_number: usize) -> Option<usize> {
        if let Some(composite) = self.view.composite() {
            composite
                .nearest_backward(line_number)
                .and_then(|ln| composite.find(ln))
        } else if line_number < self.buf.line_count() {
            Some(line_number.saturating_sub(1))
        } else {
            None
        }
    }

    pub fn update_and_view(
        &mut self,
        viewport_height: usize,
        viewport_width: usize,
    ) -> Vec<LineData<'_>> {
        self.view
            .curr_viewport
            .fit_view(viewport_height, viewport_width);
        self.view.end_index = self.visible_line_count();

        let left = self.view.curr_viewport.left();
        let cursor_state = self.cursor.state();

        self.view
            .compute_view(&self.buf)
            .map(move |line| LineData {
                line_number: line.line_number,
                data: line.data.as_str(),
                start: left,
                color: line.color,
                ty: match cursor_state {
                    Cursor::Singleton(i) => {
                        if line.index == i {
                            LineType::Origin
                        } else {
                            LineType::None
                        }
                    }
                    Cursor::Selection(start, end, _) => {
                        if !(start..=end).contains(&line.index) {
                            LineType::None
                        } else if line.index == start {
                            LineType::Origin | LineType::OriginStart
                        } else if line.index == end {
                            LineType::Origin | LineType::OriginEnd
                        } else {
                            LineType::Within
                        }
                    }
                } | if line.bookmarked {
                    LineType::Bookmarked
                } else {
                    LineType::None
                },
            })
            .collect()
    }

    pub fn filter_search(&mut self, regex: Regex) {
        self.view.compositor.filter_search(&self.buf, regex);
        self.view.invalidate_cache();
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
        if current < self.view.curr_viewport.top() {
            self.cursor.place(self.view.curr_viewport.top());
        } else if current >= self.view.curr_viewport.bottom() {
            self.cursor
                .place(self.view.curr_viewport.bottom().saturating_sub(1));
        }
    }

    pub fn move_select(&mut self, dir: Direction, select: bool, delta: ViewDelta) {
        let ndelta = match delta {
            ViewDelta::Number(n) => usize::from(n),
            ViewDelta::Page => self.view.curr_viewport.height(),
            ViewDelta::HalfPage => self.view.curr_viewport.height().div_ceil(2),
            ViewDelta::Boundary => usize::MAX,
            ViewDelta::Match => 0,
        };

        match dir {
            Direction::Back => self.cursor.back(select, |i| {
                let delta = match delta {
                    ViewDelta::Match => return self.view.compute_jump(i, dir).unwrap_or(i),
                    _ => ndelta,
                };
                i.saturating_sub(delta)
            }),
            Direction::Next => self.cursor.forward(select, |i| {
                let delta = match delta {
                    ViewDelta::Match => return self.view.compute_jump(i, dir).unwrap_or(i),
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
        self.view.curr_viewport.jump_to(i);
    }

    pub fn toggle_select_bookmarks(&mut self) {
        match self.cursor.state() {
            Cursor::Singleton(i) => {
                let line_number = self.line_at_view_index(i).unwrap();
                self.view
                    .compositor
                    .filters_mut()
                    .bookmarks_mut()
                    .toggle(line_number);
            }
            Cursor::Selection(start, end, _) => {
                for i in (start..=end).rev() {
                    let line_number = self.line_at_view_index(i).unwrap();
                    self.view
                        .compositor
                        .filters_mut()
                        .bookmarks_mut()
                        .toggle(line_number);
                }
            }
        }
        self.cursor
            .clamp(self.visible_line_count().saturating_sub(1));
        self.view.end_index = self.visible_line_count();
        self.view.invalidate_cache();
    }

    pub fn compute_jump(&self, i: usize, direction: Direction) -> Option<usize> {
        self.view.compute_jump(i, direction)
    }

    pub fn toggle_select_filters(&mut self) {
        self.view.compositor.toggle_select_filters();
        self.view.invalidate_cache();
    }

    pub fn remove_select_filter(&mut self) {
        self.view.compositor.remove_select_filters();
        self.view.invalidate_cache();
    }

    pub fn export_file(&mut self, file: File) -> Result<()> {
        self.buf.write_file(file, self.view.composite().cloned())
    }
}
