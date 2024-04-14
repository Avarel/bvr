use super::{
    cursor::{Cursor, CursorState, SelectionOrigin},
    filters::{Compositor, Filter, FilterExport},
    viewer::{CachedLine, ViewCache},
    viewport::Viewport,
};
use crate::{app::ViewDelta, colors::ColorSelector, direction::Direction};
use bvr_core::SegBuffer;
use bvr_core::{matches::CompositeStrategy, Result};
use std::fs::File;

pub struct Instance {
    name: String,
    buf: SegBuffer,
    cursor: CursorState,
    compositor: Compositor,
    view: ViewCache,
    color_selector: ColorSelector,
}

impl Instance {
    pub fn new(name: String, buf: SegBuffer) -> Self {
        let mut compositor = Compositor::new(&buf);
        let composite = compositor.create_composite();
        Self {
            view: ViewCache::new(composite),
            compositor: Compositor::new(&buf),
            name,
            buf,
            cursor: CursorState::new(),
            color_selector: ColorSelector::DEFAULT,
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

    pub fn is_following_output(&self) -> bool {
        self.view.is_following_output()
    }

    pub fn visible_line_count(&self) -> usize {
        self.view.composite().len()
    }

    pub fn compositor_mut(&mut self) -> &mut Compositor {
        &mut self.compositor
    }

    pub fn color_selector(&self) -> &ColorSelector {
        &self.color_selector
    }

    pub fn cursor(&self) -> &CursorState {
        &self.cursor
    }

    pub fn nearest_index(&self, line_number: usize) -> Option<usize> {
        self.view
            .composite()
            .nearest_backward(line_number)
            .and_then(|ln| self.view.composite().find(ln))
    }

    pub fn update_and_view(
        &mut self,
        viewport_height: usize,
        viewport_width: usize,
    ) -> impl Iterator<Item = &CachedLine> {
        self.view
            .viewport_mut()
            .fit_view(viewport_height, viewport_width);
        self.view.set_end_index(self.visible_line_count());

        self.view
            .cache_view(&self.buf, |cache| cache.color_cache(&self.compositor))
    }

    pub fn add_search_filter(&mut self, pattern: &str, literal: bool) -> Result<(), regex::Error> {
        self.compositor
            .add_search_filter(&self.buf, pattern, literal, &mut self.color_selector)?;
        self.invalidate_cache();
        Ok(())
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
        if current < self.view.viewport().top() {
            self.cursor.place(self.view.viewport().top());
        } else if current >= self.view.viewport().bottom() {
            self.cursor
                .place(self.view.viewport().bottom().saturating_sub(1));
        }
    }

    pub fn move_select(&mut self, dir: Direction, select: bool, delta: ViewDelta) {
        let compute_delta = |i: usize| match delta {
            ViewDelta::Number(n) => usize::from(n),
            ViewDelta::Page => self.view.viewport().height(),
            ViewDelta::HalfPage => self.view.viewport().height().div_ceil(2),
            ViewDelta::Boundary => usize::MAX,
            ViewDelta::Match => i.abs_diff(self.compositor.compute_jump(i, dir).unwrap_or(i)),
        };

        match dir {
            Direction::Back => self
                .cursor
                .back(select, |i| i.saturating_sub(compute_delta(i))),
            Direction::Next => self
                .cursor
                .forward(select, |i| i.saturating_add(compute_delta(i))),
        }
        self.cursor
            .clamp(self.visible_line_count().saturating_sub(1));
        let i = match self.cursor.state() {
            Cursor::Singleton(i)
            | Cursor::Selection(i, _, SelectionOrigin::Left)
            | Cursor::Selection(_, i, SelectionOrigin::Right) => i,
        };
        self.view.viewport_mut().jump_vertically_to(i);
    }

    pub fn toggle_bookmark_line_number(&mut self, line_number: usize) {
        self.compositor
            .filters_mut()
            .bookmarks_mut()
            .toggle(line_number);
        self.cursor
            .clamp(self.visible_line_count().saturating_sub(1));
        self.view.set_end_index(self.visible_line_count());

        if self
            .compositor
            .filters()
            .iter_active()
            .all(|filter| !filter.has_line(line_number))
        {
            self.invalidate_cache();
        } else {
            self.view.reset_color_cache();
        }
    }

    pub fn toggle_select_bookmarks(&mut self) {
        let mut needs_invalidation = true;
        match self.cursor.state() {
            Cursor::Singleton(i) => {
                let line_number = self.view.line_at_view_index(i).unwrap();
                return self.toggle_bookmark_line_number(line_number);
            }
            Cursor::Selection(start, end, _) => {
                let line_numbers = (start..=end)
                    .map(|i| self.view.line_at_view_index(i).unwrap())
                    .collect::<Vec<_>>();
                let present = line_numbers
                    .iter()
                    .all(|&i| self.compositor.filters().bookmarks().has_line(i));

                for line_number in line_numbers {
                    needs_invalidation = self
                        .compositor
                        .filters()
                        .iter_active()
                        .all(|filter| !filter.has_line(line_number));
                    let bookmarks = self.compositor.filters_mut().bookmarks_mut();
                    if present {
                        bookmarks.remove(line_number);
                    } else {
                        bookmarks.add(line_number);
                    }
                }
            }
        }
        self.cursor
            .clamp(self.visible_line_count().saturating_sub(1));
        self.view.set_end_index(self.visible_line_count());
        if needs_invalidation {
            self.invalidate_cache();
        } else {
            self.view.reset_color_cache();
        }
    }

    pub fn toggle_select_filters(&mut self) {
        self.compositor.toggle_select_filters();
        self.invalidate_cache();
    }

    pub fn remove_select_filter(&mut self) {
        self.compositor.remove_select_filters();
        self.invalidate_cache();
    }

    pub fn toggle_filter(&mut self, filter_index: usize) {
        self.compositor
            .filters_mut()
            .get_mut(filter_index)
            .map(Filter::toggle);
        self.invalidate_cache();
    }

    pub fn set_composite_strategy(&mut self, strategy: CompositeStrategy) {
        self.compositor.set_strategy(strategy);
        self.invalidate_cache();
    }

    pub fn export_file(&mut self, mut file: File) -> Result<()> {
        self.buf.write_to_file(&mut file, self.view.composite())
    }

    pub fn export_string(&mut self) -> Result<String> {
        let mut output = String::new();
        self.buf
            .write_to_string(&mut output, self.view.composite())?;
        output.truncate(output.trim_end_matches('\0').len());
        Ok(output)
    }

    pub fn invalidate_cache(&mut self) {
        let prev_all = self.view.composite().is_all();
        let now_all = !self.compositor.needs_composite();

        if prev_all && now_all {
            self.view.reset_color_cache();
        } else {
            self.view
                .insert_new_line_set(self.compositor.create_composite());
        }
    }

    pub fn import_user_filters(&mut self, filters: Vec<FilterExport>) {
        self.compositor.import_user_filters(&self.buf, filters);
        self.invalidate_cache();
    }
}
