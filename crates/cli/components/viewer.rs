use bvr_core::{search::BufferSearch, SegStr};
use ratatui::style::Color;
use regex::bytes::Regex;
use std::ops::Range;

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

    fn bookmarks_internal_mut(&mut self) -> &mut Bookmarks {
        match &mut self.lines {
            MaskRepr::Bookmarks(bookmarks) => bookmarks,
            _ => unreachable!(),
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
    pub masks: Vec<Mask>,
    pub mask_viewport: Viewport,
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
            masks: vec![Mask::none(), Mask::bookmark()],
            mask_viewport: Viewport::new(),
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

    pub fn update_and_mask(&mut self, viewport_height: usize) -> Vec<ViewMask> {
        let viewport = &mut self.mask_viewport;
        viewport.max_height = self.masks.len();
        viewport.height = viewport_height;

        let mut masks = Vec::with_capacity(viewport.line_range().len());

        for index in viewport.line_range() {
            let mask = &self.masks[index];
            masks.push(ViewMask {
                name: &mask.name,
                color: mask.color,
                enabled: mask.enabled,
                selected: index == viewport.current(),
            });
        }

        masks
    }

    pub fn update_and_view(&mut self, viewport_height: usize) -> Vec<ViewLine> {
        self.file.try_finalize();
        self.viewport.max_height = self.file.line_count();
        self.viewport.height = viewport_height;

        let mut lines = Vec::with_capacity(self.viewport.line_range().len());
        if self.masks[0].enabled {
            let masks = self
                .masks
                .iter()
                .filter(|mask| mask.enabled)
                .collect::<Vec<_>>();


            for line_number in self.viewport.line_range() {
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
                    selected: line_number == self.viewport.current,
                });
            }

            lines
        } else {
            let mut masks = self
                .masks
                .iter()
                .filter(|mask| mask.enabled)
                .map(|v| (0, v))
                .collect::<Vec<_>>();

            let Range { mut start, end } = self.viewport.line_range();
            // skip start lines
            let mut skipped = 0;
            while skipped < start {
                let Some((offset, _)) = masks
                    .iter_mut()
                    .filter_map(|(offset, mask)| {
                        mask.translate_to_file_line(*offset).map(|ln| (offset, ln))
                    })
                    .min_by_key(|&(_, ln)| ln)
                else {
                    break;
                };

                *offset += 1;
                skipped += 1;
            }

            while start < end {
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

                let color = masks
                    .iter()
                    .rev()
                    .find(|(_, mask)| mask.has_line(line_number))
                    .map(|(_, mask)| mask.color)
                    .unwrap_or(Color::White);

                start += 1;

                let data = self.file.get_line(line_number);

                lines.push(ViewLine {
                    line_number,
                    data,
                    color,
                    selected: line_number == self.viewport.current,
                });
            }

            lines
        }
    }

    pub fn current_selected_file_line(&self) -> usize {
        if self.masks[0].enabled {
            self.viewport.current()
        } else {
            // let vectors = self
            //     .masks
            //     .iter()
            //     .filter(|mask| mask.enabled)
            //     .map(|v| (0, v))
            //     .collect::<Vec<_>>();

            // vec![]
            todo!()
        }

        // self.translate_to_file_line(self.viewport.current())
    }

    pub fn bookmarks(&mut self) -> &mut Bookmarks {
        debug_assert!(self.masks.len() >= 2);
        self.masks[1].bookmarks_internal_mut()
    }

    pub fn clear_masks(&mut self) {
        self.masks.truncate(2);
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn current_mask_mut(&mut self) -> &mut Mask {
        &mut self.masks[self.mask_viewport.current()]
    }

    pub fn mask_search(&mut self, regex: Regex) {
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
            lines: MaskRepr::Search(SearchResults::search(&self.file, regex).unwrap()),
            color: SEARCH_COLOR_LIST
                .get(self.masks.len() - 2)
                .copied()
                .unwrap_or(Color::White),
        });
    }
}
