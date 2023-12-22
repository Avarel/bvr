use bvr_core::{search::BufferSearch, SegStr};
use ratatui::style::Color;
use regex::bytes::Regex;
use std::{ops::Range, hint::unreachable_unchecked};

use crate::direction::VDirection;

type Buffer = bvr_core::SegBuffer<bvr_core::InflightIndex>;
type SearchResults = bvr_core::search::inflight::InflightSearch;

pub struct Viewport {
    max_height: usize,
    top: usize,
    height: usize,
    current: usize,
}

impl Viewport {
    pub const fn new() -> Self {
        Self {
            max_height: 0,
            top: 0,
            height: 0,
            current: 0,
        }
    }

    pub fn height(&self) -> usize {
        self.height
    }

    fn bottom(&self) -> usize {
        self.top + self.height
    }

    fn fixup(&mut self) {
        if self.top >= self.max_height {
            self.top = self.max_height.saturating_sub(1);
        }
        if self.height > self.max_height {
            self.height = self.max_height;
        }
        if self.current >= self.max_height {
            self.current = self.max_height.saturating_sub(1);
        }
    }

    fn jump_to_current(&mut self) {
        if !(self.top..self.bottom()).contains(&self.current) {
            // height remains unchanged
            if self.top.abs_diff(self.current) < self.bottom().abs_diff(self.current) {
                // bring the top to current
                self.top = self.current;
            } else {
                // bring the bottom to current
                self.top = self.current.saturating_sub(self.height).saturating_add(1);
            }
        }
    }

    pub fn pan_view(&mut self, direction: VDirection, delta: usize) {
        self.top = match direction {
            VDirection::Up => self.top.saturating_sub(delta),
            VDirection::Down => self
                .top
                .saturating_add(delta)
                .min(self.max_height.saturating_sub(1)),
        }
    }

    pub fn move_select(&mut self, direction: VDirection, delta: usize) {
        self.current = match direction {
            VDirection::Up => self.current.saturating_sub(delta),
            VDirection::Down => self
                .current
                .saturating_add(delta)
                .min(self.max_height.saturating_sub(1)),
        };
        self.jump_to_current()
    }

    pub fn line_range(&self) -> Range<usize> {
        self.top..self.bottom().min(self.max_height)
    }

    pub fn current(&self) -> usize {
        self.current
    }
}

enum MaskRepr {
    All,
    Bookmarks(Bookmarks),
    Search(SearchResults),
}

pub struct Mask {
    name: String,
    lines: MaskRepr,
    enabled: bool,
    color: Color,
}

impl Mask {
    pub fn none() -> Self {
        Self {
            name: "All Lines".to_string(),
            lines: MaskRepr::All,
            enabled: true,
            color: Color::White,
        }
    }

    fn bookmark() -> Self {
        Self {
            name: "Bookmarks".to_string(),
            enabled: true,
            color: Color::Blue,
            lines: MaskRepr::Bookmarks(Bookmarks::new()),
        }
    }

    pub fn toggle(&mut self) {
        self.enabled = !self.enabled;
    }

    // pub fn len(&self) -> usize {
    //     match &self.lines {
    //         MaskRepr::All => unreachable!(),
    //         MaskRepr::Bookmarks(lines) => lines.len(),
    //         MaskRepr::Search(lines) => lines.len(),
    //     }
    // }

    pub fn translate_to_file_line(&self, line_number: usize) -> Option<usize> {
        match &self.lines {
            MaskRepr::All => Some(line_number),
            MaskRepr::Bookmarks(lines) => lines.get(line_number),
            MaskRepr::Search(lines) => lines.get(line_number),
        }
    }

    pub fn has_line(&self, line_number: usize) -> bool {
        match &self.lines {
            MaskRepr::All => true,
            MaskRepr::Bookmarks(lines) => lines.has_line(line_number),
            MaskRepr::Search(lines) => lines.has_line(line_number),
        }
    }
}

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

pub struct Instance {
    name: String,
    file: Buffer,
    viewport: Viewport,
    pub masks: MaskManager,
}

pub struct ViewMask<'a> {
    pub name: &'a str,
    pub color: Color,
    pub enabled: bool,
    pub selected: bool,
}

pub struct ViewLine {
    pub line_number: usize,
    pub data: SegStr,
    pub color: Color,
    pub selected: bool,
}

impl Instance {
    pub fn new(name: String, file: Buffer) -> Self {
        Self {
            name,
            file,
            viewport: Viewport::new(),
            masks: MaskManager::new(),
        }
    }

    pub fn file(&self) -> &Buffer {
        &self.file
    }

    pub fn viewport(&self) -> &Viewport {
        &self.viewport
    }

    pub fn viewport_mut(&mut self) -> &mut Viewport {
        &mut self.viewport
    }

    pub fn update_and_view(&mut self, viewport_height: usize) -> Vec<ViewLine> {
        self.file.try_finalize();
        self.viewport.height = viewport_height;

        let mut lines = Vec::with_capacity(self.viewport.line_range().len());
        if self.masks.all.enabled {
            self.viewport.max_height = self.file.line_count();
            self.viewport.fixup();
        } else {
            if self.masks.composite.is_empty() {
                self.masks.compute_composite_mask();
            }

            self.viewport.max_height = self.masks.composite.len();
            self.viewport.fixup();
        }

        let masks = self
            .masks
            .iter()
            .filter(|mask| mask.enabled)
            .collect::<Vec<_>>();

        for index in self.viewport.line_range() {
            let line_number = if self.masks.all.enabled {
                index
            } else {
                self.masks.composite[index]
            };

            let data = self.file.get_line(line_number);
            let color = masks
                .iter()
                .rev()
                .find(|mask| mask.has_line(line_number))
                .map(|mask| mask.color)
                .unwrap_or(Color::White);

            lines.push(ViewLine {
                line_number,
                data,
                color,
                selected: index == self.viewport.current,
            });
        }
        lines
    }

    pub fn current_selected_file_line(&self) -> usize {
        if self.masks.all.enabled {
            self.viewport.current()
        } else {
            self.masks.composite[self.viewport.current()]
        }
    }

    pub fn mask_search(&mut self, regex: Regex) {
        self.masks.mask_search(&self.file, regex);
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

pub struct MaskManager {
    composite: Vec<usize>,
    all: Mask,
    bookmarks: Mask,
    masks: Vec<Mask>,
    pub viewport: Viewport,
}

impl MaskManager {
    pub fn new() -> Self {
        Self {
            composite: Vec::new(),
            all: Mask::none(),
            bookmarks: Mask::bookmark(),
            masks: vec![],
            viewport: Viewport::new(),
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &Mask> {
        std::iter::once(&self.all)
            .chain(std::iter::once(&self.bookmarks))
            .chain(self.masks.iter())
    }

    pub fn update_and_mask(&mut self, viewport_height: usize) -> Vec<ViewMask> {
        let viewport = &mut self.viewport;
        viewport.max_height = self.masks.len() + 2;
        viewport.height = viewport_height;

        let mut masks = Vec::with_capacity(viewport.line_range().len());

        for index in viewport.line_range() {
            let mask = self.mask_by_index(index);
            masks.push(ViewMask {
                name: &mask.name,
                color: mask.color,
                enabled: mask.enabled,
                selected: index == self.viewport.current(),
            });
        }

        masks
    }

    pub fn mask_by_index(&self, index: usize) -> &Mask {
        match index {
            0 => &self.all,
            1 => &self.bookmarks,
            _ => &self.masks[index - 2],
        }
    }

    pub fn recompute_composite_on_next_use(&mut self) {
        self.composite.clear();
    }

    fn compute_composite_mask(&mut self) {
        let mut masks = self
            .masks
            .iter()
            .filter(|mask| mask.enabled)
            .map(|v| (0, v))
            .collect::<Vec<_>>();

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

    pub fn bookmarks(&mut self) -> &mut Bookmarks {
        // Safety: by construction
        match &mut self.bookmarks.lines {
            MaskRepr::Bookmarks(bookmarks) => bookmarks,
            _ => unsafe { unreachable_unchecked() },
        }
    }

    pub fn clear(&mut self) {
        self.masks.truncate(2);
    }

    pub fn current_mask_mut(&mut self) -> &mut Mask {
        match self.viewport.current {
            0 => &mut self.all,
            1 => &mut self.bookmarks,
            _ => &mut self.masks[self.viewport.current - 2],
        }
    }

    pub fn mask_search(&mut self, file: &Buffer, regex: Regex) {
        const SEARCH_COLOR_LIST: &[Color] = &[
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

        self.masks.push(Mask {
            name: regex.to_string(),
            enabled: true,
            lines: MaskRepr::Search(SearchResults::search(file, regex).unwrap()),
            color: SEARCH_COLOR_LIST
                .get(self.masks.len())
                .copied()
                .unwrap_or(Color::White),
        });
    }
}
