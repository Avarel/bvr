use super::{
    cursor::{Cursor, CursorState, SelectionOrigin},
    viewport::Viewport,
};
use crate::{app::ViewDelta, colors, direction::Direction, regex_compile};
use bitflags::bitflags;
use bvr_core::{matches::CompositeStrategy, LineSet, SegBuffer};
use ratatui::style::Color;
use regex::bytes::Regex;

#[derive(Clone)]
enum FilterData {
    All,
    Bookmarks(Bookmarks),
    Search(LineSet),
}

#[derive(Clone)]
pub enum Mask {
    Builtin(&'static str),
    Regex(Regex),
}

impl Mask {
    pub fn build(pattern: &str, literal: bool) -> Result<(Self, Regex), regex::Error> {
        let regex = if literal {
            regex_compile(&regex::escape(pattern))
        } else {
            regex_compile(pattern)
        }?;
        Ok((Self::Regex(regex.clone()), regex))
    }

    pub fn regex(&self) -> Option<Regex> {
        match self {
            Self::Builtin(_) => None,
            Self::Regex(regex) => Some(regex.clone()),
        }
    }
}

#[derive(Clone)]
pub struct Filter {
    mask: Mask,
    enabled: bool,
    color: Color,
    data: FilterData,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub enum MaskExport {
    Regex(String),
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct FilterExport {
    mask: MaskExport,
    enabled: bool,
    color: Color,
}

impl Filter {
    fn all() -> Self {
        Self {
            mask: Mask::Builtin("All Lines"),
            data: FilterData::All,
            enabled: true,
            color: Color::White,
        }
    }

    fn bookmark() -> Self {
        Self {
            mask: Mask::Builtin("Bookmarks"),
            enabled: true,
            color: colors::SELECT_ACCENT,
            data: FilterData::Bookmarks(Bookmarks::new()),
        }
    }

    fn from_filter(filter: Mask, color: Color, repr: FilterData) -> Self {
        Self {
            mask: filter,
            enabled: true,
            color,
            data: repr,
        }
    }

    pub fn to_export(&self) -> FilterExport {
        FilterExport {
            mask: match &self.mask {
                Mask::Regex(regex) => MaskExport::Regex(regex.to_string()),
                Mask::Builtin(_) => panic!("cannot serialize builtin mask"),
            },
            enabled: self.enabled,
            color: self.color,
        }
    }

    pub fn from_export(file: &SegBuffer, export: FilterExport) -> Self {
        let mask = match export.mask {
            MaskExport::Regex(ref regex) => Mask::Regex(regex_compile(regex).unwrap()),
        };
        Self {
            data: FilterData::Search(LineSet::search(
                file.segment_iter().unwrap(),
                mask.regex().unwrap(),
            )),
            mask,
            enabled: export.enabled,
            color: export.color,
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

    pub fn mask(&self) -> &Mask {
        &self.mask
    }

    pub fn has_line(&self, line_number: usize) -> bool {
        match &self.data {
            FilterData::All => true,
            FilterData::Bookmarks(lines) => lines.has_line(line_number),
            FilterData::Search(lines) => lines.has_line(line_number),
        }
    }

    pub fn len(&self) -> Option<usize> {
        match &self.data {
            FilterData::All => None,
            FilterData::Bookmarks(lines) => Some(lines.len()),
            FilterData::Search(lines) => Some(lines.len()),
        }
    }

    pub fn as_line_matches(&self) -> LineSet {
        match &self.data {
            FilterData::All => LineSet::empty(),
            FilterData::Bookmarks(mask) => mask.lines.clone().into(),
            FilterData::Search(mask) => mask.clone(),
        }
    }

    pub fn nearest_forward(&self, line_number: usize) -> Option<usize> {
        match &self.data {
            FilterData::All => None,
            FilterData::Bookmarks(mask) => mask.nearest_forward(line_number),
            FilterData::Search(mask) => mask.nearest_forward(line_number),
        }
    }

    pub fn nearest_backward(&self, line_number: usize) -> Option<usize> {
        match &self.data {
            FilterData::All => None,
            FilterData::Bookmarks(mask) => mask.nearest_backward(line_number),
            FilterData::Search(mask) => mask.nearest_backward(line_number),
        }
    }

    pub fn is_complete(&self) -> bool {
        match &self.data {
            FilterData::All => true,
            FilterData::Bookmarks(_) => true,
            FilterData::Search(lines) => lines.is_complete(),
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
    all: Filter,
    bookmarks: Filter,
    user_filters: Vec<Filter>,
}

impl Filters {
    fn new() -> Self {
        Self {
            all: Filter::all(),
            bookmarks: Filter::bookmark(),
            user_filters: Vec::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.user_filters.len() + 2
    }

    pub fn iter(&self) -> impl Iterator<Item = &Filter> {
        std::iter::once(&self.all)
            .chain(std::iter::once(&self.bookmarks))
            .chain(self.user_filters.iter())
    }

    pub fn iter_active(&self) -> impl Iterator<Item = &Filter> {
        self.iter().filter(|filter| filter.is_enabled())
    }

    pub fn get_mut(&mut self, index: usize) -> Option<&mut Filter> {
        match index {
            0 => Some(&mut self.all),
            1 => Some(&mut self.bookmarks),
            _ => self.user_filters.get_mut(index - 2),
        }
    }

    pub fn bookmarks(&self) -> &Bookmarks {
        // Safety: by construction
        match &self.bookmarks.data {
            FilterData::Bookmarks(bookmarks) => bookmarks,
            _ => unsafe { std::hint::unreachable_unchecked() },
        }
    }

    pub fn bookmarks_mut(&mut self) -> &mut Bookmarks {
        // Safety: by construction
        match &mut self.bookmarks.data {
            FilterData::Bookmarks(bookmarks) => bookmarks,
            _ => unsafe { std::hint::unreachable_unchecked() },
        }
    }

    pub fn clear(&mut self) {
        self.bookmarks_mut().clear();
        self.user_filters.clear();
    }
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
    ) -> impl Iterator<Item = (usize, &Filter)> {
        self.viewport.fit_view(viewport_height, 0);
        self.viewport.clamp(self.filters.len());

        self.filters
            .iter()
            .enumerate()
            .skip(self.viewport.top())
            .take(self.viewport.height())
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
                    self.filters.user_filters.remove(i - 2);
                }
            }
            Cursor::Selection(start, end, _) => {
                let start = start.max(2);
                if start <= end {
                    self.filters.user_filters.drain(start - 2..=end - 2);
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
        let (filter, regex) = Mask::build(pattern, literal)?;

        self.filters.user_filters.push(Filter::from_filter(
            filter,
            color_selector.next_color(),
            FilterData::Search(LineSet::search(file.segment_iter().unwrap(), regex)),
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

    pub fn export_user_filters(&self) -> Vec<FilterExport> {
        self.filters
            .user_filters
            .iter()
            .map(Filter::to_export)
            .collect()
    }

    pub(super) fn import_user_filters(&mut self, file: &SegBuffer, exports: Vec<FilterExport>) {
        self.filters.user_filters.extend(
            exports
                .into_iter()
                .map(|wire| Filter::from_export(file, wire)),
        );
    }

    pub fn cursor(&self) -> &CursorState {
        &self.cursor
    }
}
