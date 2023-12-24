use super::{filters::Filterer, viewport::Viewport};
use bvr_core::SegStr;
use ratatui::style::Color;
use regex::bytes::Regex;

pub type Buffer = bvr_core::SegBuffer;

pub struct Instance {
    name: String,
    file: Buffer,
    viewport: Viewport,
    pub filterer: Filterer,
}

pub struct LineData {
    pub line_number: usize,
    pub data: SegStr,
    pub start: usize,
    pub color: Color,
    pub bookmarked: bool,
    pub selected: bool,
}

impl Instance {
    pub fn new(name: String, file: Buffer) -> Self {
        Self {
            name,
            file,
            viewport: Viewport::new(),
            filterer: Filterer::new(),
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

    pub fn update_and_view(&mut self, viewport_height: usize, viewport_width: usize) -> Vec<LineData> {
        self.file.try_finalize();
        self.filterer.filters.try_finalize();
        self.filterer.composite.try_finalize();

        self.viewport.fit_view(viewport_height, viewport_width);

        let mut lines = Vec::with_capacity(self.viewport.line_range().len());
        if self.filterer.filters.all().is_enabled() {
            self.viewport.update_end(self.file.line_count());
        } else {
            self.viewport.update_end(self.filterer.composite.len());
        }

        let filters = self.filterer.filters.iter_active().collect::<Vec<_>>();

        for index in self.viewport.line_range() {
            let line_number = if self.filterer.filters.all().is_enabled() {
                index
            } else {
                self.filterer
                    .composite
                    .get(index)
                    .expect("valid index into composite")
            };

            let Some(data) = self.file.get_line(line_number) else {
                break;
            };
            let color = filters
                .iter()
                .rev()
                .find(|filter| filter.has_line(line_number))
                .map(|filter| filter.color)
                .unwrap_or(Color::White);

            let bookmarked = self.filterer.filters.bookmarks().has_line(line_number);

            lines.push(LineData {
                line_number,
                data,
                start: self.viewport.left(),
                color,
                bookmarked,
                selected: index == self.viewport.current(),
            });
        }
        lines
    }

    pub fn current_selected_file_line(&mut self) -> usize {
        if self.filterer.filters.all().is_enabled() {
            self.viewport.current()
        } else {
            self.filterer
                .composite
                .get(self.viewport.current())
                .unwrap()
        }
    }

    pub fn filter_search(&mut self, regex: Regex) {
        self.filterer.filter_search(&self.file, regex);
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}
