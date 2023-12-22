use super::{composite::inflight::InflightComposite, Buffer, SearchResults, Viewport};
use bvr_core::{
    cowvec::CowVec,
    search::{inflight::InflightSearchProgress, BufferSearch},
};
use ratatui::style::Color;
use regex::bytes::Regex;

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
            color: Color::Blue,
            repr: FilterRepr::Bookmarks(Bookmarks::new()),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn toggle(&mut self) {
        self.enabled = !self.enabled;
    }

    pub fn len(&self) -> Option<usize> {
        match &self.repr {
            FilterRepr::All => None,
            FilterRepr::Bookmarks(lines) => Some(lines.len()),
            FilterRepr::Search(lines) => Some(lines.len()),
        }
    }

    pub fn translate_to_file_line(&self, line_number: usize) -> Option<usize> {
        match &self.repr {
            FilterRepr::All => Some(line_number),
            FilterRepr::Bookmarks(lines) => lines.get(line_number),
            FilterRepr::Search(lines) => lines.get(line_number),
        }
    }

    pub fn has_line(&self, line_number: usize) -> bool {
        match &self.repr {
            FilterRepr::All => true,
            FilterRepr::Bookmarks(lines) => lines.has_line(line_number),
            FilterRepr::Search(lines) => lines.has_line(line_number),
        }
    }

    pub(crate) fn is_complete(&self) -> bool {
        match &self.repr {
            FilterRepr::All => true,
            FilterRepr::Bookmarks(_) => true,
            FilterRepr::Search(lines) => matches!(lines.progress(), InflightSearchProgress::Done),
        }
    }

    pub fn try_finalize(&mut self) {
        if let FilterRepr::Search(lines) = &mut self.repr {
            lines.try_finalize();
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

impl BufferSearch for Bookmarks {
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

impl std::ops::Index<usize> for Filters {
    type Output = Filter;

    fn index(&self, index: usize) -> &Self::Output {
        match index {
            0 => &self.all,
            1 => &self.bookmarks,
            _ => &self.searches[index - 2],
        }
    }
}

impl std::ops::IndexMut<usize> for Filters {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        match index {
            0 => &mut self.all,
            1 => &mut self.bookmarks,
            _ => &mut self.searches[index - 2],
        }
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
    const SEARCH_COLOR_LIST: &'static [Color] = &[
        Color::Red,
        Color::Green,
        Color::Yellow,
        Color::Magenta,
        Color::Cyan,
        Color::Indexed(21),
        Color::Indexed(43),
        Color::Indexed(140),
        Color::Indexed(214),
        Color::Indexed(91),
    ];

    pub fn new() -> Self {
        Self {
            composite: InflightComposite::new().0,
            viewport: Viewport::new(),
            filters: Filters::new(),
        }
    }

    pub fn update_and_filter_view(&mut self, viewport_height: usize) -> Vec<FilterData> {
        self.filters.try_finalize();

        let viewport = &mut self.viewport;
        viewport.max_height = self.filters.searches.len() + 2;
        viewport.height = viewport_height;

        let mut filters = Vec::with_capacity(viewport.line_range().len());

        for index in viewport.line_range() {
            let filter = &self.filters[index];
            filters.push(FilterData {
                name: &filter.name,
                color: filter.color,
                len: filter.len(),
                enabled: filter.enabled,
                selected: index == self.viewport.current(),
            });
        }

        filters
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
        &mut self.filters[self.viewport.current()]
    }

    pub fn filter_search(&mut self, file: &Buffer, regex: Regex) {
        self.filters.searches.push(Filter {
            name: regex.to_string(),
            enabled: true,
            repr: FilterRepr::Search(SearchResults::search(file, regex).unwrap()),
            color: Self::SEARCH_COLOR_LIST
                .get(self.filters.searches.len())
                .copied()
                .unwrap_or(Color::White),
        });
    }
}
