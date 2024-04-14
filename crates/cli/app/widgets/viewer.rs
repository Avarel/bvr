use super::super::{
    actions::{Action, NormalAction},
    mouse::MouseHandler,
    ViewDelta,
};
use crate::{
    app::actions::VisualAction,
    colors,
    components::{cursor::Cursor, instance::Instance},
    direction::Direction,
};
use bitflags::bitflags;
use crossterm::event::MouseEventKind;
use ratatui::{prelude::*, widgets::*};
use regex::bytes::Regex;

pub struct LineViewerWidget<'a> {
    pub(super) view_index: usize,
    pub(super) viewer: &'a mut Instance,
    pub(super) show_selection: bool,
    pub(super) gutter: bool,
    pub(super) regex: Option<&'a Regex>,
}

struct LineRenderData<'a> {
    line_number: usize,
    data: &'a str,
    color: Color,
    ty: LineType,
}

bitflags! {
    #[derive(Clone)]
    struct LineType: u8 {
        const None = 0;
        const Origin = 1 << 0;
        const OriginStart = 1 << 1;
        const OriginEnd = 1 << 2;
        const Within = 1 << 3;
        const Bookmarked = 1 << 4;
    }
}

impl LineViewerWidget<'_> {
    pub fn render(self, area: Rect, buf: &mut Buffer, handle: &mut MouseHandler) {
        let left = self.viewer.viewport().left();
        let search_color = self.viewer.color_selector().peek_color();
        let gutter_size = self
            .gutter
            .then(|| ((self.viewer.visible_line_count() + 1).ilog10() as u16).max(4));

        let mut itoa_buf = itoa::Buffer::new();

        let cursor_state = self.viewer.cursor().state();

        let view = self
            .viewer
            .update_and_view(area.height as usize, area.width as usize);

        (area.y..area.bottom())
            .zip(view.map(Some).chain(std::iter::repeat(None)))
            .for_each(|(y, line)| {
                ViewerLineWidget {
                    view_index: self.view_index,
                    start: left,
                    search_color,
                    line: line.map(|line| LineRenderData {
                        line_number: line.line_number,
                        data: line.data.as_str(),
                        color: line.color,
                        ty: match cursor_state {
                            Cursor::Singleton(i) => {
                                if line.index == i {
                                    LineType::Origin
                                } else {
                                    LineType::None
                                }
                            }
                            Cursor::Selection(start, end, _) => {
                                if !(start..=end).contains(&line.index) {
                                    LineType::None
                                } else if line.index == start {
                                    LineType::Origin | LineType::OriginStart
                                } else if line.index == end {
                                    LineType::Origin | LineType::OriginEnd
                                } else {
                                    LineType::Within
                                }
                            }
                        } | if line.bookmarked {
                            LineType::Bookmarked
                        } else {
                            LineType::None
                        },
                    }),
                    show_selection: self.show_selection,
                    itoa_buf: &mut itoa_buf,
                    gutter_size,
                    regex: self.regex,
                }
                .render(Rect::new(area.x, y, area.width, 1), buf, handle);
            });

        handle.on_mouse(area, |event| match event.kind {
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                Some(Action::Normal(NormalAction::PanVertical {
                    direction: Direction::back_if(event.kind == MouseEventKind::ScrollUp),
                    delta: ViewDelta::Number(5),
                    target_view: Some(self.view_index),
                }))
            }
            _ => None,
        });
    }
}

struct ViewerLineWidget<'a> {
    view_index: usize,
    line: Option<LineRenderData<'a>>,

    search_color: Color,
    itoa_buf: &'a mut itoa::Buffer,
    show_selection: bool,
    gutter_size: Option<u16>,
    start: usize,
    regex: Option<&'a Regex>,
}

impl ViewerLineWidget<'_> {
    fn gutter_selection(line: &LineRenderData) -> &'static str {
        if line.ty.contains(LineType::Origin) {
            if line.ty.contains(LineType::OriginStart) {
                "┌ "
            } else if line.ty.contains(LineType::OriginEnd) {
                "└"
            } else {
                "▶"
            }
        } else if line.ty.contains(LineType::Within) {
            "│"
        } else {
            ""
        }
    }

    fn split_line(&self, area: Rect) -> [Rect; 3] {
        const SPECIAL_SIZE: u16 = 3;
        let gutter_size = self.gutter_size.unwrap_or(0);
        let mut gutter_chunk = area;
        gutter_chunk.width = gutter_size;

        let mut type_chunk = area;
        type_chunk.x += gutter_size + 1;
        type_chunk.width = 1;

        let mut data_chunk = area;
        data_chunk.x += gutter_size + SPECIAL_SIZE;
        data_chunk.width = data_chunk.width.saturating_sub(gutter_size + SPECIAL_SIZE);

        [gutter_chunk, type_chunk, data_chunk]
    }

    pub fn render(self, area: Rect, buf: &mut Buffer, handle: &mut MouseHandler) {
        let [gutter_chunk, type_chunk, data_chunk] = self.split_line(area);

        let Some(line) = &self.line else {
            let ln = Paragraph::new("~")
                .alignment(Alignment::Right)
                .fg(colors::GUTTER_TEXT);

            ln.render(gutter_chunk, buf);
            return;
        };

        if self.gutter_size.is_some() {
            let ln_str = self.itoa_buf.format(line.line_number + 1);
            let ln = Paragraph::new(ln_str).alignment(Alignment::Right).fg(
                if line.ty.contains(LineType::Bookmarked) {
                    colors::SELECT_ACCENT
                } else {
                    colors::GUTTER_TEXT
                },
            );

            ln.render(gutter_chunk, buf);
        }

        if self.show_selection {
            Paragraph::new(Self::gutter_selection(line))
                .fg(colors::SELECT_ACCENT)
                .render(type_chunk, buf);
        }

        let mut chars = line.data.chars();
        for _ in 0..self.start {
            chars.next();
        }
        let data = &chars.as_str()
            [..(data_chunk.width as usize).min(line.data.len().saturating_sub(self.start))];

        if let Some(m) = self.regex.and_then(|r| r.find(data.as_bytes())) {
            let start = m.start();
            let end = m.end();
            let spans = vec![
                Span::raw(&data[..start]),
                Span::raw(&data[start..end]).bg(self.search_color),
                Span::raw(&data[end..]),
            ];
            Paragraph::new(Line::from(spans))
        } else {
            Paragraph::new(data)
        }
        .fg(line.color)
        .render(data_chunk, buf);

        if let Some(line) = self.line {
            handle.on_mouse(area, |event| match event.kind {
                MouseEventKind::Down(_) => Some(Action::Visual(VisualAction::ToggleLine {
                    line_number: line.line_number,
                    target_view: self.view_index,
                })),
                _ => None,
            });
        }
    }
}
