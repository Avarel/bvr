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

    pub const BLACK: Color = Color::Black;
    pub const BG: Color = Color::Rgb(25, 25, 25);
    pub const TEXT_ACTIVE: Color = Color::Rgb(220, 220, 220);
    pub const TEXT_INACTIVE: Color = Color::Rgb(50, 50, 50);
    pub const GUTTER: Color = BG;
    pub const TAB_INACTIVE: Color = Color::Rgb(50, 50, 50);
    pub const TAB_ACTIVE: Color = Color::Rgb(80, 80, 80);
    pub const STATUS_BAR: Color = Color::Rgb(40, 40, 40);
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
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(9), Constraint::Min(1)])
            .split(area);

        match self.state {
            StatusWidgetState::Normal {
                progress,
                line_count,
                name,
            } => {
                let chunks = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([
                        Constraint::Min(1),
                        Constraint::Length(name.len() as u16 + 2),
                    ])
                    .split(chunks[1]);
                Paragraph::new(Span::from(match progress {
                    InflightIndexProgress::Done => format!("{} lines", line_count),
                    InflightIndexProgress::Streaming => format!("Streaming ({} lines)", line_count),
                    InflightIndexProgress::File(progress) => {
                        format!("{:.2}% ({} lines)", progress * 100.0, line_count)
                    }
                }))
                .block(Block::new().padding(Padding::horizontal(1)))
                .dark_gray()
                .bg(colors::STATUS_BAR)
                .render(chunks[0], buf);

                Paragraph::new(name)
                    .block(Block::new().padding(Padding::horizontal(1)))
                    .alignment(Alignment::Right)
                    .bg(colors::STATUS_BAR)
                    .render(chunks[1], buf);
            }
            StatusWidgetState::Message { message } => {
                Paragraph::new(message)
                    .block(Block::new().padding(Padding::horizontal(1)))
                    .dark_gray()
                    .on_black()
                    .render(chunks[1], buf);
            }
            StatusWidgetState::None => {
                Paragraph::new("Open a file with :open [filename]")
                    .block(Block::new().padding(Padding::horizontal(1)))
                    .dark_gray()
                    .on_black()
                    .render(chunks[1], buf);
            }
        }

        let mode_tag = match self.input_mode {
            InputMode::Command => Paragraph::new(Span::from("COMMAND")).on_light_green(),
            InputMode::Viewer => Paragraph::new(Span::from("VIEWER")).on_blue(),
            InputMode::Select => Paragraph::new(Span::from("SELECT")).on_magenta(),
        };

        mode_tag
            .block(Block::new().padding(Padding::horizontal(1)))
            .render(chunks[0], buf);
    }
}

pub struct CommandWidget<'a> {
    pub inner: &'a CommandApp,
    pub cursor: &'a mut Option<(u16, u16)>,
    pub active: bool,
}

impl Widget for CommandWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let command_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(1), Constraint::Min(1)])
            .split(area);

        let input = Paragraph::new({
            let mut v = Vec::new();

            match *self.inner.cursor() {
                Cursor::Singleton(_) => v.push(Span::from(self.inner.buf())),
                Cursor::Selection(start, end, _) => {
                    v.push(Span::from(&self.inner.buf()[..start]));
                    v.push(Span::from(&self.inner.buf()[start..end]).on_blue());
                    v.push(Span::from(&self.inner.buf()[end..]));
                }
            }

            Line::from(v)
        })
        .style(match self.active {
            false => Style::default(),
            true => Style::default().fg(Color::Yellow),
        });
        match self.active {
            false => {}
            true => {
                Paragraph::new(":").render(command_chunks[0], buf);
                match *self.inner.cursor() {
                    Cursor::Singleton(i) => {
                        *self.cursor = Some((command_chunks[1].x + i as u16, command_chunks[1].y));
                    }
                    Cursor::Selection(start, end, dir) => {
                        let x = match dir {
                            SelectionOrigin::Right => end,
                            SelectionOrigin::Left => start,
                        };
                        *self.cursor = Some((command_chunks[1].x + x as u16, command_chunks[1].y));
                    }
                }
            }
        }
        input.render(command_chunks[1], buf);
    }
}

pub struct ViewerWidget<'a> {
    viewer: &'a mut Instance,
}

fn digit_count(n: usize) -> u16 {
    n.ilog10() as u16 + 1
}

impl Widget for ViewerWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        self.viewer
            .viewport_mut()
            .fit_view(usize::from(area.height));

        let view = self.viewer.update_and_view();

        let gutter_size = view
            .iter()
            .map(|line| digit_count(line.line_number() + 1))
            .max()
            .unwrap_or(0)
            .max(4);

        let mut y = area.y;
        for line in view.into_iter() {
            LineWidget { line, gutter_size }.render(Rect::new(area.x, y, area.width, 1), buf);
            y += 1;
        }

        while y < area.bottom() {
            EmptyLineWidget { gutter_size }.render(Rect::new(area.x, y, area.width, 1), buf);
            y += 1;
        }
    }
}

struct LineWidget {
    gutter_size: u16,
    line: ViewLine,
}

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
    .padding(Padding::new(0, 0, 0, 0))
    .border_set(SET_LEFT_EDGE)
    .border_style(Style::new().fg(colors::BLACK))
    .borders(Borders::LEFT);

impl Widget for LineWidget {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let chunks = Layout::new()
            .constraints([Constraint::Length(self.gutter_size + 1), Constraint::Min(1)])
            .direction(Direction::Horizontal)
            .split(area);

        let ln_str = (self.line.line_number() + 1).to_string();
        let ln = Paragraph::new(ln_str)
            .block(LINE_WIDGET_BLOCK)
            .alignment(Alignment::Right)
            .fg(colors::TEXT_INACTIVE)
            .bg(colors::GUTTER);
        ln.render(chunks[0], buf);

        let data = Paragraph::new(self.line.data().as_str())
            .block(Block::new().padding(Padding::new(3, 0, 0, 0)))
            .bg(colors::BG);
        data.render(chunks[1], buf);
    }
}

struct EmptyLineWidget {
    gutter_size: u16,
}

impl Widget for EmptyLineWidget {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let chunks = Layout::new()
            .constraints([Constraint::Length(self.gutter_size + 1), Constraint::Min(1)])
            .direction(Direction::Horizontal)
            .split(area);

        let ln = Paragraph::new("~")
            .block(LINE_WIDGET_BLOCK)
            .alignment(Alignment::Right)
            .fg(colors::TEXT_INACTIVE)
            .bg(colors::GUTTER);
        ln.render(chunks[0], buf);

        let data = Paragraph::new("").bg(colors::BG);
        data.render(chunks[1], buf);
    }
}

pub struct MultiplexerWidget<'a> {
    pub mux: &'a mut MultiplexerApp,
    pub status: &'a mut StatusApp,
    pub(super) mode: InputMode,
}

impl MultiplexerWidget<'_> {
    fn split_status(area: Rect) -> std::rc::Rc<[Rect]> {
        Layout::new()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(area)
    }

    fn split_tabs(area: Rect) -> std::rc::Rc<[Rect]> {
        Layout::new()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(1)])
            .split(area)
    }

    fn split_horizontal(area: Rect, len: usize) -> std::rc::Rc<[Rect]> {
        let constraints = vec![Constraint::Ratio(1, len as u32); len];
        Layout::new()
            .direction(Direction::Horizontal)
            .constraints(constraints)
            .split(area)
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
                Span::from("▍ ").fg(colors::TEXT_ACTIVE)
            } else {
                Span::from("▏ ").fg(colors::GUTTER)
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
                        ViewerWidget { viewer }.render(vsplit[1], buf);
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
                    ViewerWidget { viewer }.render(vsplit[1], buf);
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
