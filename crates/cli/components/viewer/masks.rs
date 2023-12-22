use super::{Buffer, SearchResults, Viewport};
use bvr_core::{
    cowvec::CowVec,
    search::{inflight::InflightSearchProgress, BufferSearch},
};
use ratatui::style::Color;
use regex::bytes::Regex;

#[derive(Clone)]
enum MaskRepr {
    All,
    Bookmarks(Bookmarks),
    Search(SearchResults),
}

#[derive(Clone)]
pub struct Mask {
    name: String,
    enabled: bool,
    pub(super) color: Color,
    repr: MaskRepr,
}

impl Mask {
    fn all() -> Self {
        Self {
            name: "All Lines".to_string(),
            repr: MaskRepr::All,
            enabled: true,
            color: Color::White,
        }
    }

    fn bookmark() -> Self {
        Self {
            name: "Bookmarks".to_string(),
            enabled: true,
            color: Color::Blue,
            repr: MaskRepr::Bookmarks(Bookmarks::new()),
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
            MaskRepr::All => None,
            MaskRepr::Bookmarks(lines) => Some(lines.len()),
            MaskRepr::Search(lines) => Some(lines.len()),
        }
    }

    pub fn translate_to_file_line(&self, line_number: usize) -> Option<usize> {
        match &self.repr {
            MaskRepr::All => Some(line_number),
            MaskRepr::Bookmarks(lines) => lines.get(line_number),
            MaskRepr::Search(lines) => lines.get(line_number),
        }
    }

    pub fn has_line(&self, line_number: usize) -> bool {
        match &self.repr {
            MaskRepr::All => true,
            MaskRepr::Bookmarks(lines) => lines.has_line(line_number),
            MaskRepr::Search(lines) => lines.has_line(line_number),
        }
    }

    pub(crate) fn is_complete(&self) -> bool {
        match &self.repr {
            MaskRepr::All => true,
            MaskRepr::Bookmarks(_) => true,
            MaskRepr::Search(lines) => matches!(lines.progress(), InflightSearchProgress::Done),
        }
    }

    pub fn try_finalize(&mut self) {
        match &mut self.repr {
            MaskRepr::Search(lines) => {
                lines.try_finalize();
            }
            _ => {}
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

pub struct Masker {
    pub(crate) composite: Vec<usize>,
    pub viewport: Viewport,
    pub(crate) masks: Masks,
}

#[derive(Clone)]
pub struct Masks {
    all: Mask,
    bookmarks: Mask,
    searches: Vec<Mask>,
}

impl Masks {
    fn new() -> Self {
        Self {
            all: Mask::all(),
            bookmarks: Mask::bookmark(),
            searches: Vec::new(),
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &Mask> {
        std::iter::once(&self.all)
            .chain(std::iter::once(&self.bookmarks))
            .chain(self.searches.iter())
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Mask> {
        std::iter::once(&mut self.all)
            .chain(std::iter::once(&mut self.bookmarks))
            .chain(self.searches.iter_mut())
    }

    pub fn try_finalize(&mut self) {
        for mask in self.iter_mut() {
            mask.try_finalize();
        }
    }

    pub fn iter_active(&self) -> impl Iterator<Item = &Mask> {
        self.iter().filter(|mask| mask.is_enabled())
    }

    pub fn all(&self) -> &Mask {
        &self.all
    }

    #[allow(dead_code)]
    pub fn all_mut(&mut self) -> &mut Mask {
        &mut self.all
    }

    #[allow(dead_code)]
    pub fn bookmarks(&self) -> &Bookmarks {
        // Safety: by construction
        match &self.bookmarks.repr {
            MaskRepr::Bookmarks(bookmarks) => bookmarks,
            _ => unsafe { std::hint::unreachable_unchecked() },
        }
    }

    pub fn bookmarks_mut(&mut self) -> &mut Bookmarks {
        // Safety: by construction
        match &mut self.bookmarks.repr {
            MaskRepr::Bookmarks(bookmarks) => bookmarks,
            _ => unsafe { std::hint::unreachable_unchecked() },
        }
    }

    pub fn clear(&mut self) {
        self.searches.truncate(2);
    }
}

impl std::ops::Index<usize> for Masks {
    type Output = Mask;

    fn index(&self, index: usize) -> &Self::Output {
        match index {
            0 => &self.all,
            1 => &self.bookmarks,
            _ => &self.searches[index - 2],
        }
    }
}

impl std::ops::IndexMut<usize> for Masks {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        match index {
            0 => &mut self.all,
            1 => &mut self.bookmarks,
            _ => &mut self.searches[index - 2],
        }
    }
}

pub struct MaskData<'a> {
    pub name: &'a str,
    pub color: Color,
    pub len: Option<usize>,
    pub enabled: bool,
    pub selected: bool,
}

impl Masker {
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
            composite: Vec::new(),
            viewport: Viewport::new(),
            masks: Masks::new(),
        }
    }

    pub fn update_and_mask(&mut self, viewport_height: usize) -> Vec<MaskData> {
        let viewport = &mut self.viewport;
        viewport.max_height = self.masks.searches.len() + 2;
        viewport.height = viewport_height;

        let mut masks = Vec::with_capacity(viewport.line_range().len());

        for index in viewport.line_range() {
            let mask = &self.masks[index];
            masks.push(MaskData {
                name: &mask.name,
                color: mask.color,
                len: mask.len(),
                enabled: mask.enabled,
                selected: index == self.viewport.current(),
            });
        }

        masks
    }

    pub fn recompute_composite_on_next_use(&mut self) {
        self.composite.clear();
    }

    pub(crate) fn compute_composite_mask(&mut self) {
        let mut masks = self.masks.iter_active().map(|v| (0, v)).collect::<Vec<_>>();

        loop {
            let Some((offset, line_number)) = masks
                .iter_mut()
                .filter_map(|(offset, mask)| {
                    mask.translate_to_file_line(*offset).map(|ln| (offset, ln))
                })
                .min_by_key(|&(_, ln)| ln)
            else {
                break;
            };

            *offset += 1;

            self.composite.push(line_number);
        }
    }

    pub fn current_mask_mut(&mut self) -> &mut Mask {
        &mut self.masks[self.viewport.current()]
    }

    pub fn mask_search(&mut self, file: &Buffer, regex: Regex) {
        self.masks.searches.push(Mask {
            name: regex.to_string(),
            enabled: true,
            repr: MaskRepr::Search(SearchResults::search(file, regex).unwrap()),
            color: Self::SEARCH_COLOR_LIST
                .get(self.masks.searches.len())
                .copied()
                .unwrap_or(Color::White),
        });
    }
}
