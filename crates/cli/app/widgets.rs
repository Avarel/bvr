use crate::{
    colors,
    components::{
        command::{CommandApp, Cursor, SelectionOrigin},
        filters::FilterData,
        mux::{MultiplexerApp, MultiplexerMode},
        status::StatusApp,
        viewer::{Instance, LineData},
    },
    direction::VDirection,
};
use crossterm::event::MouseEventKind;
use ratatui::{prelude::*, widgets::*};

use super::{
    actions::{Action, Delta, ViewerAction},
    mouse::MouseHandler,
    InputMode,
};

enum StatusWidgetState<'a> {
    Normal { line_count: usize, name: &'a str },
    Message { message: &'a str },
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
            InputMode::Viewer => colors::VIEWER_ACCENT,
            InputMode::Select => colors::SELECT_ACCENT,
            InputMode::Filter => colors::FILTER_ACCENT,
        };

        let mut v = Vec::new();

        v.push(
            Span::from(match self.input_mode {
                InputMode::Command => " COMMAND ",
                InputMode::Viewer => " VIEWER ",
                InputMode::Select => " SELECT ",
                InputMode::Filter => " FILTER ",
            })
            .fg(colors::WHITE)
            .bg(accent_color),
        );
        v.push(Span::raw(" "));

        match self.state {
            StatusWidgetState::Normal { line_count, name } => {
                v.push(Span::raw(format!("{} lines", line_count)).fg(accent_color));
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
            const WIDGET_BLOCK: Block = Block::new().style(Style::new().bg(colors::BG));
            WIDGET_BLOCK.render(area, buf);
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
            let i = match *self.inner.cursor() {
                Cursor::Singleton(i) => i,
                Cursor::Selection(start, _, SelectionOrigin::Right) => start,
                Cursor::Selection(_, end, SelectionOrigin::Left) => end,
            };
            *self.cursor = Some((area.x + i as u16 + 1, area.y));
        }
        input.render(area, buf);
    }
}

pub struct FilterViewerWidget<'a> {
    viewer: &'a mut Instance,
}

impl FilterViewerWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        const WIDGET_BLOCK: Block = Block::new().style(Style::new().bg(colors::STATUS_BAR));
        WIDGET_BLOCK.render(area, buf);

        let mut y = area.y;
        for filter in self
            .viewer
            .filterer
            .update_and_filter_view(area.height as usize)
        {
            FilterLineWidget { inner: &filter }.render(Rect::new(area.x, y, area.width, 1), buf);
            y += 1;
        }
    }
}

pub struct ViewerWidget<'a> {
    view_index: usize,
    viewer: &'a mut Instance,
    gutter: bool,
}

impl ViewerWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer, handle: &mut MouseHandler) {
        let view = self.viewer.update_and_view(area.height as usize);

        let gutter_size = self.gutter.then(|| {
            view.last()
                .map(|ln| ((ln.line_number + 1).ilog10() + 1) as u16)
                .unwrap_or_default()
                .max(4)
        });

        let mut itoa_buf = itoa::Buffer::new();
        let mut y = area.y;
        for line in view.into_iter() {
            ViewerLineWidget {
                view_index: self.view_index,
                line: Some(line),
                itoa_buf: &mut itoa_buf,
                gutter_size,
            }
            .render(Rect::new(area.x, y, area.width, 1), buf, handle);
            y += 1;
        }

        while y < area.bottom() {
            ViewerLineWidget {
                view_index: self.view_index,
                line: None,
                itoa_buf: &mut itoa_buf,
                gutter_size,
            }
            .render(Rect::new(area.x, y, area.width, 1), buf, handle);
            y += 1;
        }

        handle.on_mouse(area, |event| match event.kind {
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                Some(Action::Viewer(ViewerAction::Pan {
                    direction: VDirection::up_if(event.kind == MouseEventKind::ScrollUp),
                    delta: Delta::Number(2),
                    target_view: Some(self.view_index),
                }))
            }
            // MouseEventKind::Down(_) => Some(Action::Viewer(ViewerAction::SwitchActiveIndex(
            //     self.view_index,
            // ))),
            _ => None,
        });
    }
}

struct EdgeBg(bool);

impl EdgeBg {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if self.0 {
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

struct FilterLineWidget<'a> {
    inner: &'a FilterData<'a>,
}

impl FilterLineWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let mut v = vec![
            Span::from(if self.inner.selected { " ▶" } else { "  " }).fg(colors::FILTER_ACCENT),
            Span::from(if self.inner.enabled { " ● " } else { " ◯ " }).fg(self.inner.color),
            Span::from(self.inner.name).fg(self.inner.color),
        ];

        if let Some(len) = self.inner.len {
            v.push(Span::from(format!(" {}", len)).fg(colors::TEXT_INACTIVE));
        }

        Paragraph::new(Line::from(v)).render(area, buf);
    }
}

struct ViewerLineWidget<'a> {
    view_index: usize,
    itoa_buf: &'a mut itoa::Buffer,
    gutter_size: Option<u16>,
    line: Option<LineData>,
}

impl ViewerLineWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer, handle: &mut MouseHandler) {
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

        if self.gutter_size.is_some() {
            if let Some(line) = &self.line {
                let ln_str = self.itoa_buf.format(line.line_number + 1);
                let ln =
                    Paragraph::new(ln_str)
                        .alignment(Alignment::Right)
                        .fg(if line.bookmarked {
                            colors::SELECT_ACCENT
                        } else {
                            colors::GUTTER_TEXT
                        });

                ln.render(gutter_chunk, buf);

                if line.selected {
                    let ln = Paragraph::new("▶").fg(colors::SELECT_ACCENT);
                    ln.render(type_chunk, buf);
                }

                let data = Paragraph::new(line.data.as_str()).fg(line.color);
                data.render(data_chunk, buf);
            } else {
                let ln = Paragraph::new("~")
                    .alignment(Alignment::Right)
                    .fg(colors::GUTTER_TEXT);

                ln.render(gutter_chunk, buf);
            }
        } else if let Some(line) = &self.line {
            if line.selected {
                let ln = Paragraph::new("▶").fg(colors::SELECT_ACCENT);
                ln.render(type_chunk, buf);
            } else if line.bookmarked {
                let ln = Paragraph::new("▸").fg(colors::SELECT_ACCENT);
                ln.render(type_chunk, buf);
            }

            let data = Paragraph::new(line.data.as_str()).fg(line.color);

            data.render(data_chunk, buf);
        }

        if let Some(line) = self.line {
            handle.on_mouse(area, |event| match event.kind {
                MouseEventKind::Down(_) => Some(Action::Viewer(ViewerAction::ToggleLine {
                    line_number: line.line_number,
                    target_view: self.view_index,
                })),
                _ => None,
            });
        }
    }
}

pub struct TabWidget<'a> {
    view_index: usize,
    name: &'a str,
    active: bool,
}

impl TabWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer, handle: &mut MouseHandler) {
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

        handle.on_mouse(area, |event| match event.kind {
            MouseEventKind::Down(_) => Some(Action::Viewer(ViewerAction::SwitchActiveIndex(
                self.view_index,
            ))),
            _ => None,
        });
    }
}

pub struct MultiplexerWidget<'a> {
    pub mux: &'a mut MultiplexerApp,
    pub status: &'a mut StatusApp,
    pub(super) mode: InputMode,
}

impl MultiplexerWidget<'_> {
    pub fn split_status(area: Rect) -> [Rect; 2] {
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

    fn split_filter(area: Rect) -> [Rect; 2] {
        const FILTER_MAX_HEIGHT: u16 = 10;

        let mut view_chunk = area;
        view_chunk.height = view_chunk.height.saturating_sub(FILTER_MAX_HEIGHT);

        let mut filter_chunk = area;
        filter_chunk.y = area.y + view_chunk.height;
        filter_chunk.height = FILTER_MAX_HEIGHT.min(area.height);

        [view_chunk, filter_chunk]
    }
}

impl MultiplexerWidget<'_> {
    pub fn render(self, area: Rect, buf: &mut Buffer, handler: &mut MouseHandler) {
        let [mux_chunk, status_chunk] = Self::split_status(area);

        fn fixup_chunk(fix: bool, mut chunk: Rect) -> Rect {
            if fix {
                chunk.x += 1;
                chunk.width -= 1;
            }
            chunk
        }

        if !self.mux.is_empty() {
            let active = self.mux.active();
            match self.mux.mode() {
                MultiplexerMode::Windows => {
                    let hsplit = Self::split_horizontal(mux_chunk, self.mux.len());

                    for (i, (&chunk, viewer)) in
                        hsplit.iter().zip(self.mux.viewers_mut()).enumerate()
                    {
                        let [tab_chunk, view_chunk] = Self::split_tabs(chunk);
                        TabWidget {
                            view_index: i,
                            name: viewer.name(),
                            active: active == i,
                        }
                        .render(tab_chunk, buf, handler);

                        let mut viewer_chunk = view_chunk;

                        if self.mode == InputMode::Filter {
                            let [view_chunk, filter_chunk] = Self::split_filter(view_chunk);
                            FilterViewerWidget { viewer }.render(filter_chunk, buf);
                            viewer_chunk = view_chunk;
                        }

                        ViewerWidget {
                            view_index: i,
                            viewer,
                            gutter: true,
                        }
                        .render(
                            fixup_chunk(i != 0, viewer_chunk),
                            buf,
                            handler,
                        );
                        EdgeBg(i == 0).render(viewer_chunk, buf)
                    }
                }
                MultiplexerMode::Tabs => {
                    let [tab_chunk, view_chunk] = Self::split_tabs(mux_chunk);
                    let hsplit = Self::split_horizontal(tab_chunk, self.mux.len());

                    for (i, (&chunk, viewer)) in
                        hsplit.iter().zip(self.mux.viewers_mut()).enumerate()
                    {
                        TabWidget {
                            view_index: i,
                            name: viewer.name(),
                            active: active == i,
                        }
                        .render(chunk, buf, handler);
                    }

                    let active = self.mux.active();
                    let viewer = self.mux.active_viewer_mut().unwrap();
                    let mut viewer_chunk = view_chunk;

                    if self.mode == InputMode::Filter {
                        let [view_chunk, filter_chunk] = Self::split_filter(view_chunk);
                        FilterViewerWidget { viewer }.render(filter_chunk, buf);
                        viewer_chunk = view_chunk;
                    }
                    ViewerWidget {
                        view_index: active,
                        viewer,
                        gutter: true,
                    }
                    .render(viewer_chunk, buf, handler);
                    EdgeBg(true).render(viewer_chunk, buf)
                }
            }
        }

        match self.status.get_message_update() {
            Some(ref message) => StatusWidget {
                input_mode: self.mode,
                state: StatusWidgetState::Message { message },
            }
            .render(status_chunk, buf),
            None => match self.mux.active_viewer_mut() {
                Some(viewer) => StatusWidget {
                    input_mode: self.mode,
                    state: StatusWidgetState::Normal {
                        line_count: viewer.file().line_count(),
                        name: viewer.name(),
                    },
                }
                .render(status_chunk, buf),
                None => StatusWidget {
                    input_mode: self.mode,
                    state: StatusWidgetState::None,
                }
                .render(status_chunk, buf),
            },
        }
    }
}
