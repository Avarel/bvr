use super::{
    cursor::{Cursor, CursorState, SelectionOrigin},
    viewport::Viewport,
};
use crate::{app::ViewDelta, colors, direction::Direction};
use bitflags::bitflags;
use bvr_core::{matches::CompositeStrategy, LineSet, SegBuffer};
use ratatui::style::Color;
use regex::bytes::Regex;

#[derive(Clone)]
enum FilterRepr {
    All,
    Bookmarks(Bookmarks),
    Search(LineSet),
}

#[derive(Clone)]
pub struct Filter {
    name: String,
    enabled: bool,
    color: Color,
    repr: FilterRepr,
}

impl Filter {
    fn all() -> Self {
        Self {
            name: "All Lines".to_string(),
            repr: FilterRepr::All,
            enabled: true,
            color: Color::White,
        }
    }

    fn bookmark() -> Self {
        Self {
            name: "Bookmarks".to_string(),
            enabled: true,
            color: colors::SELECT_ACCENT,
            repr: FilterRepr::Bookmarks(Bookmarks::new()),
        }
    }

    pub fn color(&self) -> Color {
        self.color
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn toggle(&mut self) {
        self.enabled = !self.enabled;
    }

    pub fn has_line(&self, line_number: usize) -> bool {
        match &self.repr {
            FilterRepr::All => true,
            FilterRepr::Bookmarks(lines) => lines.has_line(line_number),
            FilterRepr::Search(lines) => lines.has_line(line_number),
        }
    }

    fn len(&self) -> Option<usize> {
        match &self.repr {
            FilterRepr::All => None,
            FilterRepr::Bookmarks(lines) => Some(lines.len()),
            FilterRepr::Search(lines) => Some(lines.len()),
        }
    }

    pub fn as_line_matches(&self) -> LineSet {
        match &self.repr {
            FilterRepr::All => LineSet::empty(),
            FilterRepr::Bookmarks(mask) => mask.lines.clone().into(),
            FilterRepr::Search(mask) => mask.clone(),
        }
    }

    pub fn nearest_forward(&self, line_number: usize) -> Option<usize> {
        match &self.repr {
            FilterRepr::All => None,
            FilterRepr::Bookmarks(mask) => mask.nearest_forward(line_number),
            FilterRepr::Search(mask) => mask.nearest_forward(line_number),
        }
    }

    pub fn nearest_backward(&self, line_number: usize) -> Option<usize> {
        match &self.repr {
            FilterRepr::All => None,
            FilterRepr::Bookmarks(mask) => mask.nearest_backward(line_number),
            FilterRepr::Search(mask) => mask.nearest_backward(line_number),
        }
    }

    pub fn is_complete(&self) -> bool {
        match &self.repr {
            FilterRepr::All => true,
            FilterRepr::Bookmarks(_) => true,
            FilterRepr::Search(lines) => lines.is_complete(),
        }
    }
}

#[derive(Clone)]
pub struct Bookmarks {
    lines: Vec<usize>,
}

impl Bookmarks {
    fn new() -> Bookmarks {
        Bookmarks { lines: Vec::new() }
    }

    pub fn toggle(&mut self, line_number: usize) {
        match self.lines.binary_search(&line_number) {
            Ok(idx) => {
                self.lines.remove(idx);
            }
            Err(idx) => {
                self.lines.insert(idx, line_number);
            }
        };
    }

    pub fn has_line(&self, line_number: usize) -> bool {
        let slice = self.lines.as_slice();
        match *slice {
            [first, .., last] if (first..=last).contains(&line_number) => {
                slice.binary_search(&line_number).is_ok()
            }
            [item] => item == line_number,
            _ => false,
        }
    }

    fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn nearest_forward(&self, line_number: usize) -> Option<usize> {
        let slice = self.lines.as_slice();
        match *slice {
            [_, ..] => Some(
                slice[match slice.binary_search(&line_number) {
                    Ok(i) => i.saturating_add(1),
                    Err(i) => i,
                }
                .min(slice.len() - 1)],
            ),
            [] => None,
        }
    }

    pub fn nearest_backward(&self, line_number: usize) -> Option<usize> {
        let slice = self.lines.as_slice();
        match *slice {
            [_, ..] => Some(
                slice[match slice.binary_search(&line_number) {
                    Ok(i) | Err(i) => i,
                }
                .saturating_sub(1)
                .min(slice.len() - 1)],
            ),
            [] => None,
        }
    }

    fn clear(&mut self) {
        self.lines.clear()
    }
}

#[derive(Clone)]
pub struct Filters {
    all: Filter,
    bookmarks: Filter,
    searches: Vec<Filter>,
}

impl Filters {
    fn new() -> Self {
        Self {
            all: Filter::all(),
            bookmarks: Filter::bookmark(),
            searches: Vec::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.searches.len() + 2
    }

    pub fn iter(&self) -> impl Iterator<Item = &Filter> {
        std::iter::once(&self.all)
            .chain(std::iter::once(&self.bookmarks))
            .chain(self.searches.iter())
    }

    pub fn iter_active(&self) -> impl Iterator<Item = &Filter> {
        self.iter().filter(|filter| filter.is_enabled())
    }

    pub fn get_mut(&mut self, index: usize) -> Option<&mut Filter> {
        match index {
            0 => Some(&mut self.all),
            1 => Some(&mut self.bookmarks),
            _ => self.searches.get_mut(index - 2),
        }
    }

    pub fn bookmarks(&self) -> &Bookmarks {
        // Safety: by construction
        match &self.bookmarks.repr {
            FilterRepr::Bookmarks(bookmarks) => bookmarks,
            _ => unsafe { std::hint::unreachable_unchecked() },
        }
    }

    pub fn bookmarks_mut(&mut self) -> &mut Bookmarks {
        // Safety: by construction
        match &mut self.bookmarks.repr {
            FilterRepr::Bookmarks(bookmarks) => bookmarks,
            _ => unsafe { std::hint::unreachable_unchecked() },
        }
    }

    pub fn clear(&mut self) {
        self.bookmarks_mut().clear();
        self.searches.clear();
    }
}

bitflags! {
    pub struct FilterType: u8 {
        const None = 0;
        const Enabled = 1 << 0;
        const Origin = 1 << 1;
        const OriginStart = 1 << 2;
        const OriginEnd = 1 << 3;
        const Within = 1 << 4;
    }
}

pub struct FilterData<'a> {
    pub index: usize,
    pub name: &'a str,
    pub color: Color,
    pub len: Option<usize>,
    pub ty: FilterType,
}

pub struct Compositor {
    all_composite: LineSet,
    strategy: CompositeStrategy,
    viewport: Viewport,
    cursor: CursorState,
    filters: Filters,
}

impl Compositor {
    pub fn new(buf: &SegBuffer) -> Self {
        Self {
            all_composite: buf.all_line_matches(),
            viewport: Viewport::new(),
            cursor: CursorState::new(),
            filters: Filters::new(),
            strategy: CompositeStrategy::Union,
        }
    }

    pub fn set_strategy(&mut self, strategy: CompositeStrategy) {
        self.strategy = strategy;
    }

    pub fn needs_composite(&self) -> bool {
        !self.filters.all.is_enabled()
    }

    pub fn filters_mut(&mut self) -> &mut Filters {
        &mut self.filters
    }

    pub fn filters(&self) -> &Filters {
        &self.filters
    }

    pub fn update_and_filter_view(
        &mut self,
        viewport_height: usize,
    ) -> impl Iterator<Item = FilterData> {
        self.viewport.fit_view(viewport_height, 0);
        self.viewport.clamp(self.filters.len());

        self.filters
            .iter()
            .enumerate()
            .skip(self.viewport.top())
            .take(self.viewport.height())
            .map(|(index, filter)| FilterData {
                index,
                name: &filter.name,
                color: filter.color,
                len: filter.len(),
                ty: match self.cursor.state() {
                    Cursor::Singleton(i) => {
                        if index == i {
                            FilterType::Origin
                        } else {
                            FilterType::None
                        }
                    }
                    Cursor::Selection(start, end, _) => {
                        if !(start..=end).contains(&index) {
                            FilterType::None
                        } else if index == start {
                            FilterType::Origin | FilterType::OriginStart
                        } else if index == end {
                            FilterType::Origin | FilterType::OriginEnd
                        } else {
                            FilterType::Within
                        }
                    }
                } | if filter.enabled {
                    FilterType::Enabled
                } else {
                    FilterType::None
                },
            })
    }

    pub fn create_composite(&self) -> LineSet {
        if self.filters.all.is_enabled() {
            self.all_composite.clone()
        } else {
            let filters = self
                .filters
                .iter_active()
                .map(|filter| filter.as_line_matches())
                .collect();
            LineSet::compose(filters, false, self.strategy).unwrap()
        }
    }

    pub fn move_select(&mut self, dir: Direction, select: bool, delta: ViewDelta) {
        let delta = match delta {
            ViewDelta::Number(n) => usize::from(n),
            ViewDelta::Page => self.viewport.height(),
            ViewDelta::HalfPage => self.viewport.height().div_ceil(2),
            ViewDelta::Boundary => usize::MAX,
            ViewDelta::Match => unimplemented!("there is no result jumping for filters"),
        };
        match dir {
            Direction::Back => self.cursor.back(select, |i| i.saturating_sub(delta)),
            Direction::Next => self.cursor.forward(select, |i| i.saturating_add(delta)),
        }
        self.cursor.clamp(self.filters.len().saturating_sub(1));
        let i = match self.cursor.state() {
            Cursor::Singleton(i)
            | Cursor::Selection(i, _, SelectionOrigin::Left)
            | Cursor::Selection(_, i, SelectionOrigin::Right) => i,
        };
        self.viewport.jump_to(i);
    }

    pub fn toggle_select_filters(&mut self) {
        match self.cursor.state() {
            Cursor::Singleton(i) => {
                self.filters.get_mut(i).map(Filter::toggle);
            }
            Cursor::Selection(start, end, _) => {
                for i in start..=end {
                    self.filters.get_mut(i).map(Filter::toggle);
                }
            }
        }
    }

    pub fn remove_select_filters(&mut self) {
        match self.cursor.state() {
            Cursor::Singleton(i) => {
                if i > 1 {
                    self.filters.searches.remove(i - 2);
                }
            }
            Cursor::Selection(start, end, _) => {
                let start = start.max(2);
                if start <= end {
                    self.filters.searches.drain(start - 2..=end - 2);
                }
            }
        }
        self.cursor.clamp(self.filters.len().saturating_sub(1));
    }

    pub fn filter_search(&mut self, file: &SegBuffer, regex: Regex) {
        self.filters.searches.push(Filter {
            name: regex.to_string(),
            enabled: true,
            repr: FilterRepr::Search(LineSet::search(file.segment_iter().unwrap(), regex)),
            color: colors::SEARCH_COLOR_LIST
                .get(self.filters.searches.len())
                .copied()
                .unwrap_or(Color::White),
        });
    }
}
