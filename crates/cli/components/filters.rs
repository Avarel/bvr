use crate::colors;

use super::viewer::{Buffer, Viewport};
use bvr_core::{composite::inflight::InflightComposite, cowvec::CowVec, matches::BufferMatches};
use ratatui::style::Color;
use regex::bytes::Regex;

type SearchResults = bvr_core::InflightSearch;

#[derive(Clone)]
enum FilterRepr {
    All,
    Bookmarks(Bookmarks),
    Search(SearchResults),
}

#[derive(Clone)]
pub struct Filter {
    name: String,
    enabled: bool,
    pub(super) color: Color,
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

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn toggle(&mut self) {
        self.enabled = !self.enabled;
    }

    pub fn try_finalize(&mut self) {
        if let FilterRepr::Search(lines) = &mut self.repr {
            lines.try_finalize();
        }
    }
}

impl BufferMatches for Filter {
    fn get(&self, index: usize) -> Option<usize> {
        match &self.repr {
            FilterRepr::All => Some(index),
            FilterRepr::Bookmarks(lines) => lines.get(index),
            FilterRepr::Search(lines) => lines.get(index),
        }
    }

    fn has_line(&self, line_number: usize) -> bool {
        match &self.repr {
            FilterRepr::All => true,
            FilterRepr::Bookmarks(lines) => lines.has_line(line_number),
            FilterRepr::Search(lines) => lines.has_line(line_number),
        }
    }

    fn len(&self) -> usize {
        match &self.repr {
            FilterRepr::All => 0,
            FilterRepr::Bookmarks(lines) => lines.len(),
            FilterRepr::Search(lines) => lines.len(),
        }
    }

    fn is_complete(&self) -> bool {
        match &self.repr {
            FilterRepr::All | FilterRepr::Bookmarks(_) => true,
            FilterRepr::Search(search) => search.is_complete(),
        }
    }
}

#[derive(Clone)]
pub struct Bookmarks {
    lines: CowVec<usize>,
}

impl Bookmarks {
    fn new() -> Bookmarks {
        Bookmarks {
            lines: CowVec::new(),
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
}

impl BufferMatches for Bookmarks {
    fn get(&self, index: usize) -> Option<usize> {
        self.lines.get(index).copied()
    }

    fn has_line(&self, line_number: usize) -> bool {
        let slice = self.lines.as_slice();
        if let &[first, .., last] = slice {
            if (first..=last).contains(&line_number) {
                return slice.binary_search(&line_number).is_ok();
            }
        } else if let &[item] = slice {
            return item == line_number;
        }
        false
    }

    fn len(&self) -> usize {
        self.lines.len()
    }

    fn is_complete(&self) -> bool {
        true
    }
}

pub struct Filterer {
    pub(super) composite: InflightComposite,
    pub viewport: Viewport,
    pub(crate) filters: Filters,
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

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Filter> {
        std::iter::once(&mut self.all)
            .chain(std::iter::once(&mut self.bookmarks))
            .chain(self.searches.iter_mut())
    }

    pub fn try_finalize(&mut self) {
        for filter in self.iter_mut() {
            filter.try_finalize();
        }
    }

    pub fn iter_active(&self) -> impl Iterator<Item = &Filter> {
        self.iter().filter(|filter| filter.is_enabled())
    }

    pub fn all(&self) -> &Filter {
        &self.all
    }

    #[allow(dead_code)]
    pub fn all_mut(&mut self) -> &mut Filter {
        &mut self.all
    }

    #[allow(dead_code)]
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
        self.searches.clear();
    }
}

pub struct FilterData<'a> {
    pub name: &'a str,
    pub color: Color,
    pub len: Option<usize>,
    pub enabled: bool,
    pub selected: bool,
}

impl Filterer {
    pub fn new() -> Self {
        Self {
            composite: InflightComposite::empty(),
            viewport: Viewport::new(),
            filters: Filters::new(),
        }
    }

    pub fn update_and_filter_view(&mut self, viewport_height: usize) -> Vec<FilterData> {
        self.filters.try_finalize();

        let viewport = &mut self.viewport;
        viewport.max_height = self.filters.searches.len() + 2;
        viewport.height = viewport_height;

        let range = viewport.line_range();

        self.filters
            .iter()
            .enumerate()
            .skip(range.start)
            .take(range.len())
            .map(|(index, filter)| FilterData {
                name: &filter.name,
                color: filter.color,
                len: match &filter.repr {
                    FilterRepr::All => None,
                    _ => Some(filter.len()),
                },
                enabled: filter.enabled,
                selected: index == self.viewport.current(),
            })
            .collect()
    }

    pub fn compute_composite(&mut self) {
        if self.filters.all().is_enabled() {
            self.composite = InflightComposite::empty();
            return;
        }
        let (composite, remote) = InflightComposite::new();
        std::thread::spawn({
            let filters = self.filters.iter_active().cloned().collect();
            move || {
                remote.compute(filters).unwrap();
            }
        });
        self.composite = composite;
    }

    pub fn current_filter_mut(&mut self) -> &mut Filter {
        assert!(self.viewport.current() < self.filters.len());

        match self.viewport.current() {
            0 => &mut self.filters.all,
            1 => &mut self.filters.bookmarks,
            _ => &mut self.filters.searches[self.viewport.current() - 2],
        }
    }

    pub fn remove_current_filter(&mut self) {
        assert!(self.viewport.current() < self.filters.len());

        match self.viewport.current() {
            0 => self.filters.all.enabled = false,
            1 => self.filters.bookmarks.enabled = false,
            _ => {
                self.filters.searches.remove(self.viewport.current() - 2);
            }
        }
    }

    pub fn filter_search(&mut self, file: &Buffer, regex: Regex) {
        self.filters.searches.push(Filter {
            name: regex.to_string(),
            enabled: true,
            repr: FilterRepr::Search(SearchResults::search(file, regex).unwrap()),
            color: colors::SEARCH_COLOR_LIST
                .get(self.filters.searches.len())
                .copied()
                .unwrap_or(Color::White),
        });
    }
}
