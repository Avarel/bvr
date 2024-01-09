use super::{filters::Compositor, viewport::Viewport};
use bvr_core::{LineSet, SegBuffer, SegStr};
use ratatui::style::Color;
use std::collections::VecDeque;

#[derive(Clone)]
pub struct CachedLine {
    pub index: usize,
    pub line_number: usize,
    pub data: SegStr,
    pub color: Color,
    pub bookmarked: bool,
}

pub struct ViewCache {
    composite: LineSet,
    cache: VecDeque<CachedLine>,

    prev_viewport: Viewport,
    curr_viewport: Viewport,

    follow_output: bool,
    end_index: usize,

    need_recoloring: bool,
}

impl ViewCache {
    pub(crate) fn new(composite: LineSet) -> Self {
        Self {
            composite,
            cache: VecDeque::new(),
            prev_viewport: Viewport::new(),
            curr_viewport: Viewport::new(),
            follow_output: false,
            need_recoloring: false,
            end_index: 0,
        }
    }

    pub fn set_end_index(&mut self, end_index: usize) {
        self.end_index = end_index;
    }

    pub fn composite(&self) -> &LineSet {
        &self.composite
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

    pub fn line_at_view_index(&self, index: usize) -> Option<usize> {
        self.composite.get(index)
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

    pub fn cache_view(
        &mut self,
        buf: &SegBuffer,
        preprocess: impl FnOnce(&mut Self),
    ) -> (impl Iterator<Item = &CachedLine>, Option<usize>) {
        if self.follow_output {
            self.curr_viewport.jump_to(self.end_index.saturating_sub(1));
        }

        self.curr_viewport.clamp(self.end_index);

        let (old_top, new_top) = (self.prev_viewport.top(), self.curr_viewport.top());
        let (old_bot, new_bot) = (self.prev_viewport.bottom(), self.curr_viewport.bottom());


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
            } else {
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

        self.prev_viewport = self.curr_viewport;

        preprocess(self);

        (
            self.cache.iter(),
            self.cache.back().map(|line| line.line_number),
        )
    }

    pub fn color_cache(&mut self, compositor: &Compositor) {
        if self.need_recoloring {
            self.reset_color_cache();
            self.need_recoloring = compositor
                .filters()
                .iter_active()
                .any(|filter| !filter.is_complete());
        }

        let filters = compositor.filters().iter_active().collect::<Vec<_>>();

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

                line.bookmarked = compositor.filters().bookmarks().has_line(line.line_number);
            });
    }

    pub fn reset_color_cache(&mut self) {
        self.need_recoloring = true;
        self.cache
            .iter_mut()
            .for_each(|line| line.color = Color::Reset);
    }

    pub fn insert_new_line_set(&mut self, line_set: LineSet) {
        self.cache.clear();
        let old_line_number = self.line_at_view_index(self.curr_viewport.top());
        self.composite = line_set;
        if let Some(old_line_number) = old_line_number {
            if let Some(index) = self.composite.find(old_line_number) {
                self.curr_viewport.top_to(index);
            }
        }
    }

    pub fn is_following_output(&self) -> bool {
        self.follow_output
    }
}
