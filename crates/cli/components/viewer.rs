use bvr_core::{search::BufferSearch, SegStr};
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
    const fn new() -> Self {
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

    pub fn fit_view(&mut self, height: usize) {
        self.height = height;
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
    Passthrough,
    Manual(Vec<usize>),
    Search(SearchResults),
}

pub struct Mask {
    lines: MaskRepr,
    viewport: Viewport,
    // _settings: HashMap<usize, ()>,
}

impl Mask {
    pub fn new() -> Self {
        Self {
            lines: MaskRepr::Passthrough,
            viewport: Viewport::new(),
            // _settings: HashMap::new(),
        }
    }

    pub fn toggle(&mut self, line_number: usize) {
        match &mut self.lines {
            MaskRepr::Passthrough => (),
            MaskRepr::Manual(lines) => {
                match lines.binary_search(&line_number) {
                    Ok(idx) => {
                        lines.remove(idx);
                    }
                    Err(idx) => {
                        lines.insert(idx, line_number);
                    }
                };
                self.viewport.max_height = lines.len();
            },
            MaskRepr::Search(_) => {}
        }
    }

    pub fn len(&self) -> usize {
        match &self.lines {
            MaskRepr::Passthrough => self.viewport.max_height,
            MaskRepr::Manual(lines) => lines.len(),
            MaskRepr::Search(lines) => lines.len(),
        }
    }

    pub fn translate_to_file_line(&self, line_number: usize) -> Option<usize> {
        match &self.lines {
            MaskRepr::Passthrough => Some(line_number),
            MaskRepr::Manual(lines) => lines.get(line_number).copied(),
            MaskRepr::Search(lines) => lines.get(line_number),
        }
    }

    pub fn has_line(&self, line_number: usize) -> bool {
        match &self.lines {
            MaskRepr::Passthrough => true,
            MaskRepr::Manual(lines) => {
                if let &[first, .., last] = lines.as_slice() {
                    if (first..=last).contains(&line_number) {
                        return lines.binary_search(&line_number).is_ok();
                    }
                } else if let &[item] = lines.as_slice() {
                    return item == line_number;
                }
                false
            }
            MaskRepr::Search(lines) => lines.has_line(line_number),
        }
    }
}

pub struct Instance {
    name: String,
    file: Buffer,
    mask: Vec<Mask>,
    pub view_index: usize,
}

pub struct ViewLine {
    line_number: usize,
    data: SegStr,
    line_type: LineType,
}

impl ViewLine {
    pub fn line_number(&self) -> usize {
        self.line_number
    }

    pub fn data(&self) -> &SegStr {
        &self.data
    }

    pub(crate) fn line_type(&self) -> LineType {
        self.line_type
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LineType {
    Plain,
    Selected,
    Mask,
}

impl Instance {
    pub fn new(name: String, file: Buffer) -> Self {
        Self {
            name,
            file,
            mask: vec![Mask::new()],
            view_index: 0,
        }
    }

    pub fn file(&self) -> &Buffer {
        &self.file
    }

    pub fn viewport(&self) -> &Viewport {
        &self.mask[self.view_index].viewport
    }

    pub fn viewport_mut(&mut self) -> &mut Viewport {
        &mut self.mask[self.view_index].viewport
    }

    pub fn update_and_view(&mut self) -> Vec<ViewLine> {
        self.file.try_finalize();
        self.mask[0].viewport.max_height = self.file.line_count();

        let mask = &self.mask[self.view_index];
        let mask_after = self.mask.get(self.view_index + 1);

        mask.viewport
            .line_range()
            .map(|idx| (idx, mask.translate_to_file_line(idx).unwrap()))
            .map(|(idx, line_number)| ViewLine {
                line_number,
                data: self.file.get_line(line_number),
                line_type: if idx == mask.viewport.current {
                    LineType::Selected
                } else if mask_after
                    .map(|mask| mask.has_line(line_number))
                    .unwrap_or(false)
                {
                    LineType::Mask
                } else {
                    LineType::Plain
                },
            })
            .collect()
    }

    pub fn mask_manual(&mut self) -> &mut Mask {
        if !matches!(self.mask.last().unwrap().lines, MaskRepr::Manual(_)) {
            self.mask.push(Mask {
                lines: MaskRepr::Manual(Vec::new()),
                viewport: Viewport::new(),
            });
        }
        self.mask.last_mut().unwrap()
    }

    pub fn clear_mask(&mut self) {
        self.mask.truncate(1);
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn mask_search(&mut self, regex: Regex) {
        self.mask.push(Mask {
            lines: MaskRepr::Search(SearchResults::search(&self.file, regex).unwrap()),
            viewport: Viewport::new(),
        });
    }
}
