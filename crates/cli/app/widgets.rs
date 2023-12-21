use crate::components::{
    command::{CommandApp, Cursor, SelectionOrigin},
    mux::{MultiplexerApp, MultiplexerMode},
    status::StatusApp,
    viewer::{Instance, ViewLine},
};
use bvr_core::index::inflight::InflightIndexProgress;
use ratatui::{prelude::*, widgets::*};

use super::InputMode;

mod colors {
    use ratatui::style::Color;

    pub const WHITE: Color = Color::Rgb(255, 255, 255);
    pub const BLACK: Color = Color::Rgb(0, 0, 0);
    pub const BG: Color = Color::Rgb(25, 25, 25);

    pub const TEXT_ACTIVE: Color = Color::Rgb(220, 220, 220);
    pub const TEXT_INACTIVE: Color = Color::Rgb(50, 50, 50);

    pub const GUTTER_BG: Color = BG;
    pub const GUTTER_TEXT: Color = Color::Rgb(40, 40, 40);

    pub const TAB_INACTIVE: Color = Color::Rgb(40, 40, 40);
    pub const TAB_ACTIVE: Color = Color::Rgb(80, 80, 80);
    pub const TAB_SIDE_ACTIVE: Color = Color::Blue;
    pub const TAB_SIDE_INACTIVE: Color = Color::Black;

    pub const STATUS_BAR: Color = Color::Rgb(40, 40, 40);
    pub const STATUS_BAR_TEXT: Color = Color::Rgb(150, 150, 150);

    pub const COMMAND_BAR_SELECT: Color = Color::Rgb(60, 80, 150);

    pub const COMMAND_ACCENT: Color = Color::Rgb(100, 230, 160);
    pub const SELECT_ACCENT: Color = Color::Rgb(180, 130, 230);
    pub const NORMAL_ACCENT: Color = Color::Rgb(100, 160, 230);
    pub const MASK_ACCENT: Color = Color::Rgb(255, 200, 60);
}

enum StatusWidgetState<'a> {
    Normal {
        progress: InflightIndexProgress,
        line_count: usize,
        name: &'a str,
    },
    Message {
        message: &'a str,
    },
    None,
}

pub struct StatusWidget<'a> {
    input_mode: InputMode,
    state: StatusWidgetState<'a>,
}

impl<'a> Widget for StatusWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        const STATUS_BAR_STYLE: Style = Style::new()
            .fg(colors::STATUS_BAR_TEXT)
            .bg(colors::STATUS_BAR);

        let accent_color = match self.input_mode {
            InputMode::Command => colors::COMMAND_ACCENT,
            InputMode::Viewer => colors::NORMAL_ACCENT,
            InputMode::Select => colors::SELECT_ACCENT,
        };

        let mut v = Vec::new();

        v.push(
            Span::from(match self.input_mode {
                InputMode::Command => " COMMAND ",
                InputMode::Viewer => " VIEWER ",
                InputMode::Select => " SELECT ",
            })
            .fg(colors::WHITE)
            .bg(accent_color),
        );
        v.push(Span::raw(" "));

        match self.state {
            StatusWidgetState::Normal {
                progress,
                line_count,
                name,
            } => {
                v.push(
                    Span::raw(match progress {
                        InflightIndexProgress::Done => format!("{} lines", line_count),
                        InflightIndexProgress::Streaming => {
                            format!("Streaming ({} lines)", line_count)
                        }
                        InflightIndexProgress::File(progress) => {
                            format!("{:.2}% ({} lines)", progress * 100.0, line_count)
                        }
                    })
                    .fg(accent_color),
                );
                v.push(Span::raw(" │ ").fg(accent_color));
                v.push(Span::raw(name).fg(accent_color));
            }
            StatusWidgetState::Message { message } => v.push(Span::raw(message)),
            StatusWidgetState::None => {
                v.push(Span::raw("Open a file with :open [filename]").fg(accent_color))
            }
        }

        Paragraph::new(Line::from(v))
            .style(STATUS_BAR_STYLE)
            .render(area, buf);
    }
}

pub struct CommandWidget<'a> {
    pub inner: &'a CommandApp,
    pub cursor: &'a mut Option<(u16, u16)>,
    pub active: bool,
}

impl Widget for CommandWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if !self.active {
            return;
        }

        let input = Paragraph::new(Line::from(match *self.inner.cursor() {
            Cursor::Singleton(_) => {
                vec![Span::from(":"), Span::from(self.inner.buf())]
            }
            Cursor::Selection(start, end, _) => vec![
                Span::from(":"),
                Span::from(&self.inner.buf()[..start]),
                Span::from(&self.inner.buf()[start..end]).bg(colors::COMMAND_BAR_SELECT),
                Span::from(&self.inner.buf()[end..]),
            ],
        }))
        .bg(colors::BG);

        if self.active {
            match *self.inner.cursor() {
                Cursor::Singleton(i) => {
                    *self.cursor = Some((area.x + i as u16 + 1, area.y));
                }
                Cursor::Selection(start, end, dir) => {
                    let x = match dir {
                        SelectionOrigin::Right => end,
                        SelectionOrigin::Left => start,
                    };
                    *self.cursor = Some((area.x + x as u16 + 1, area.y));
                }
            }
        }
        input.render(area, buf);
    }
}

pub struct ViewerWidget<'a> {
    viewer: &'a mut Instance,
    gutter: bool,
    first: bool,
}

impl Widget for ViewerWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        self.viewer
            .viewport_mut()
            .fit_view(usize::from(area.height));

        let view = self.viewer.update_and_view();

        let gutter_size = self.gutter.then(|| {
            view.last()
                .map(|ln| ((ln.line_number() + 1).ilog10() + 1) as u16)
                .unwrap_or_default()
                .max(4)
        });

        let mut y = area.y;
        for line in view.into_iter() {
            LineWidget {
                line: Some(line),
                gutter_size,
            }
            .render(Rect::new(area.x, y, area.width, 1), buf);
            y += 1;
        }

        while y < area.bottom() {
            LineWidget {
                line: None,
                gutter_size,
            }
            .render(Rect::new(area.x, y, area.width, 1), buf);
            y += 1;
        }

        if self.first {
            const WIDGET_BLOCK: Block = Block::new()
                .border_style(Style::new().fg(colors::BLACK).bg(colors::GUTTER_BG))
                .style(Style::new().bg(colors::BG));

            WIDGET_BLOCK.render(area, buf);
        } else {
            const SET_LEFT_EDGE: symbols::border::Set = symbols::border::Set {
                top_left: "",
                top_right: "",
                bottom_left: "",
                bottom_right: "",
                vertical_left: "▏",
                vertical_right: "",
                horizontal_top: "",
                horizontal_bottom: "",
            };

            const LINE_WIDGET_BLOCK: Block = Block::new()
                .border_set(SET_LEFT_EDGE)
                .border_style(Style::new().fg(colors::BLACK).bg(colors::GUTTER_BG))
                .borders(Borders::LEFT)
                .style(Style::new().bg(colors::BG));

            LINE_WIDGET_BLOCK.render(area, buf);
        }
    }
}

struct LineWidget {
    gutter_size: Option<u16>,
    line: Option<ViewLine>,
}

impl Widget for LineWidget {
    fn render(self, area: Rect, buf: &mut Buffer) {
        const SPECIAL_SIZE: u16 = 3;

        let gutter_size = self.gutter_size.unwrap_or(0);
        let mut gutter_chunk = area;
        gutter_chunk.width = gutter_size + 1;

        let mut type_chunk = area;
        type_chunk.x += gutter_size + 2;
        type_chunk.width = 1;

        let mut data_chunk = area;
        data_chunk.x += gutter_size + 1 + SPECIAL_SIZE;
        data_chunk.width = data_chunk
            .width
            .saturating_sub(gutter_size + SPECIAL_SIZE + 1);

        if self.gutter_size.is_some() {
            if let Some(line) = self.line {
                let ln_str = (line.line_number() + 1).to_string();
                let ln = Paragraph::new(ln_str)
                    .alignment(Alignment::Right)
                    .fg(colors::GUTTER_TEXT);

                ln.render(gutter_chunk, buf);

                if line.selected() {
                    let ln = Paragraph::new("▶").fg(colors::SELECT_ACCENT);
                    ln.render(type_chunk, buf);
                }

                let data = Paragraph::new(line.data().as_str()).fg(line.color());
                data.render(data_chunk, buf);
            } else {
                let ln = Paragraph::new("~")
                    .alignment(Alignment::Right)
                    .fg(colors::GUTTER_TEXT);

                ln.render(gutter_chunk, buf);
            }
        } else {
            if let Some(line) = self.line {
                if line.selected() {
                    let ln = Paragraph::new("▶").fg(colors::SELECT_ACCENT);
                    ln.render(type_chunk, buf);
                }

                let data = Paragraph::new(line.data().as_str());

                data.render(area, buf);
            }
        }
    }
}

pub struct MultiplexerWidget<'a> {
    pub mux: &'a mut MultiplexerApp,
    pub status: &'a mut StatusApp,
    pub(super) mode: InputMode,
}

impl MultiplexerWidget<'_> {
    fn split_status(area: Rect) -> [Rect; 2] {
        let mut status_chunk = area;
        status_chunk.y = status_chunk.bottom().saturating_sub(1);
        status_chunk.height = 1;

        let mut data_chunk = area;
        data_chunk.height = data_chunk.height.saturating_sub(1);

        [data_chunk, status_chunk]
    }

    fn split_tabs(area: Rect) -> [Rect; 2] {
        let mut tab_chunk = area;
        tab_chunk.height = 1;

        let mut data_chunk = area;
        data_chunk.y += 1;
        data_chunk.height = data_chunk.height.saturating_sub(1);

        [tab_chunk, data_chunk]
    }

    fn split_horizontal(area: Rect, len: usize) -> std::rc::Rc<[Rect]> {
        let constraints = vec![Constraint::Ratio(1, len as u32); len];
        Layout::new(Direction::Horizontal, constraints).split(area)
    }
}

pub struct TabWidget<'a> {
    name: &'a str,
    active: bool,
}

impl Widget for TabWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        Paragraph::new(Line::from(vec![
            if self.active {
                Span::from("▍ ").fg(colors::TAB_SIDE_ACTIVE)
            } else {
                Span::from("▏ ").fg(colors::TAB_SIDE_INACTIVE)
            },
            Span::from(self.name),
        ]))
        .bg(if self.active {
            colors::TAB_ACTIVE
        } else {
            colors::TAB_INACTIVE
        })
        .fg(if self.active {
            colors::TEXT_ACTIVE
        } else {
            colors::TEXT_INACTIVE
        })
        .render(area, buf);
    }
}

impl Widget for MultiplexerWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let chunks = Self::split_status(area);

        if !self.mux.is_empty() {
            let active = self.mux.active();
            match self.mux.mode() {
                MultiplexerMode::Windows => {
                    let hsplit = Self::split_horizontal(chunks[0], self.mux.len());

                    for (i, (&chunk, viewer)) in
                        hsplit.iter().zip(self.mux.viewers_mut()).enumerate()
                    {
                        let vsplit = Self::split_tabs(chunk);
                        TabWidget {
                            name: viewer.name(),
                            active: active == i,
                        }
                        .render(vsplit[0], buf);
                        ViewerWidget {
                            viewer,
                            gutter: true,
                            first: i == 0,
                        }
                        .render(vsplit[1], buf);
                    }
                }
                MultiplexerMode::Tabs => {
                    let vsplit = Self::split_tabs(chunks[0]);
                    let hsplit = Self::split_horizontal(vsplit[0], self.mux.len());

                    for (i, (&chunk, viewer)) in
                        hsplit.iter().zip(self.mux.viewers_mut()).enumerate()
                    {
                        TabWidget {
                            name: viewer.name(),
                            active: active == i,
                        }
                        .render(chunk, buf);
                    }

                    let viewer = self.mux.active_viewer_mut().unwrap();
                    ViewerWidget {
                        viewer,
                        gutter: true,
                        first: true,
                    }
                    .render(vsplit[1], buf);
                }
            }
        }

        match self.status.get_message_update() {
            Some(message) => StatusWidget {
                input_mode: self.mode,
                state: StatusWidgetState::Message { message: &message },
            }
            .render(chunks[1], buf),
            None => match self.mux.active_viewer_mut() {
                Some(viewer) => StatusWidget {
                    input_mode: self.mode,
                    state: StatusWidgetState::Normal {
                        progress: viewer.file().progress(),
                        line_count: viewer.file().line_count(),
                        name: viewer.name(),
                    },
                }
                .render(chunks[1], buf),
                None => StatusWidget {
                    input_mode: self.mode,
                    state: StatusWidgetState::None,
                }
                .render(chunks[1], buf),
            },
        }
    }
}
