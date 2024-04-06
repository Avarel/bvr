use super::{
    cursor::{Cursor, CursorState, SelectionOrigin},
    viewport::Viewport,
};
use crate::{app::ViewDelta, colors, direction::Direction};
use bitflags::bitflags;
use bvr_core::{matches::CompositeStrategy, LineSet, SegBuffer};
use ratatui::style::Color;
use regex::bytes::{Regex, RegexBuilder};

#[derive(Clone)]
enum FilterRepr {
    All,
    Bookmarks(Bookmarks),
    Search(LineSet),
}

#[derive(Clone)]
pub enum Filter {
    Builtin(&'static str),
    Literal(String, Regex),
    Regex(Regex),
}

impl Filter {
    pub fn build(pattern: &str, literal: bool) -> Result<(Self, Regex), regex::Error> {
        if literal {
            let regex = RegexBuilder::new(&regex::escape(pattern))
                .case_insensitive(true)
                .build()?;
            Ok((Self::Literal(pattern.to_owned(), regex.clone()), regex))
        } else {
            let regex = RegexBuilder::new(pattern).case_insensitive(true).build()?;
            Ok((Self::Regex(regex.clone()), regex))
        }
    }

    pub fn regex(&self) -> Option<Regex> {
        match self {
            Self::Builtin(_) => None,
            Self::Literal(_, regex) | Self::Regex(regex) => Some(regex.clone()),
        }
    }
}

#[derive(Clone)]
pub struct FilterState {
    filter: Filter,
    enabled: bool,
    color: Color,
    repr: FilterRepr,
}

impl FilterState {
    fn all() -> Self {
        Self {
            filter: Filter::Builtin("All Lines"),
            repr: FilterRepr::All,
            enabled: true,
            color: Color::White,
        }
    }

    fn bookmark() -> Self {
        Self {
            filter: Filter::Builtin("Bookmarks"),
            enabled: true,
            color: colors::SELECT_ACCENT,
            repr: FilterRepr::Bookmarks(Bookmarks::new()),
        }
    }

    fn from_filter(filter: Filter, color: Color, repr: FilterRepr) -> Self {
        Self {
            filter,
            enabled: true,
            color,
            repr,
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

    pub fn add(&mut self, line_number: usize) {
        if let Err(idx) = self.lines.binary_search(&line_number) {
            self.lines.insert(idx, line_number);
        }
    }

    pub fn remove(&mut self, line_number: usize) {
        if let Ok(idx) = self.lines.binary_search(&line_number) {
            self.lines.remove(idx);
        }
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
    all: FilterState,
    bookmarks: FilterState,
    searches: Vec<FilterState>,
}

impl Filters {
    fn new() -> Self {
        Self {
            all: FilterState::all(),
            bookmarks: FilterState::bookmark(),
            searches: Vec::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.searches.len() + 2
    }

    pub fn iter(&self) -> impl Iterator<Item = &FilterState> {
        std::iter::once(&self.all)
            .chain(std::iter::once(&self.bookmarks))
            .chain(self.searches.iter())
    }

    pub fn iter_active(&self) -> impl Iterator<Item = &FilterState> {
        self.iter().filter(|filter| filter.is_enabled())
    }

    pub fn get_mut(&mut self, index: usize) -> Option<&mut FilterState> {
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
    pub name: &'a Filter,
    pub color: Color,
    pub len: Option<usize>,
    pub ty: FilterType,
}

// #[derive(Clone, Hash, PartialEq, Eq)]
// struct CacheKey(Box<[usize]>, CompositeStrategy);

// impl CacheKey {
//     fn new(filters: &Filters, strategy: CompositeStrategy) -> Self {
//         let mut filter_ids = filters
//             .iter_active()
//             .map(|filter| filter.id)
//             .collect::<Vec<_>>();
//         filter_ids.sort();
//         Self(filter_ids.into_boxed_slice(), strategy)
//     }

//     fn contains(&self, filter: &FilterState) -> bool {
//         self.0.contains(&filter.id)
//     }
// }

pub struct Compositor {
    // id_source: usize,
    all_composite: LineSet,
    // composite_cache: LruCache<CacheKey, LineSet>,
    strategy: CompositeStrategy,
    viewport: Viewport,
    cursor: CursorState,
    filters: Filters,
}

impl Compositor {
    pub fn new(buf: &SegBuffer) -> Self {
        Self {
            // id_source: 2,
            all_composite: buf.all_line_matches(),
            // composite_cache: LruCache::new(NonZeroUsize::new(8).unwrap()),
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
                name: &filter.filter,
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

    pub fn create_composite(&mut self) -> LineSet {
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
        self.viewport.jump_vertically_to(i);
    }

    pub fn toggle_select_filters(&mut self) {
        match self.cursor.state() {
            Cursor::Singleton(i) => {
                self.filters.get_mut(i).map(FilterState::toggle);
            }
            Cursor::Selection(start, end, _) => {
                for i in start..=end {
                    self.filters.get_mut(i).map(FilterState::toggle);
                }
            }
        }
    }

    pub fn remove_select_filters(&mut self) {
        match self.cursor.state() {
            Cursor::Singleton(i) => {
                if i > 1 {
                    let filter = self.filters.searches.remove(i - 2);
                    
                }
            }
            Cursor::Selection(start, end, _) => {
                let start = start.max(2);
                if start <= end {
                    let filters = self
                        .filters
                        .searches
                        .drain(start - 2..=end - 2)
                        .collect::<Vec<_>>();
                }
            }
        }
        self.cursor.clamp(self.filters.len().saturating_sub(1));
    }

    pub fn add_search_filter(
        &mut self,
        file: &SegBuffer,
        pattern: &str,
        literal: bool,
        color_selector: &mut colors::ColorSelector,
    ) -> Result<(), regex::Error> {
        let (filter, regex) = Filter::build(pattern, literal)?;

        self.filters.searches.push(FilterState::from_filter(
            filter,
            color_selector.next_color(),
            FilterRepr::Search(LineSet::search(file.segment_iter().unwrap(), regex)),
        ));
        Ok(())
    }

    pub fn compute_jump(&self, i: usize, direction: Direction) -> Option<usize> {
        // TODO: jump to next matching filter
        if !self.filters.all.is_enabled() {
            return match direction {
                Direction::Back => Some(i.saturating_sub(1)),
                Direction::Next => Some(i + 1),
            };
        }
        let active_filters = self.filters.iter_active();
        match direction {
            Direction::Back => active_filters
                .filter_map(|filter| filter.nearest_backward(i))
                .filter(|&ln| ln < i)
                .max(),
            Direction::Next => active_filters
                .filter_map(|filter| filter.nearest_forward(i))
                .filter(|&ln| ln > i)
                .min(),
        }
    }
}
