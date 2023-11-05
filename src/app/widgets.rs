use crate::ui::{
    command::{CommandApp, Cursor, SelectionOrigin},
    viewer::{Multiplexer, MultiplexerMode, Viewer},
};
use ratatui::{prelude::*, widgets::*};

use super::InputMode;

pub struct StatusWidget<'a> {
    pub(super) input_mode: InputMode,
    pub progress: f64,
    pub line_count: usize,
    pub name: &'a str,
}

impl<'a> Widget for StatusWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(11),
                Constraint::Min(1),
                Constraint::Length(self.name.len() as u16 + 2),
            ])
            .split(area);

        Paragraph::new(Span::from(if self.progress > 1.0 {
            format!("100% ({} lines)", self.line_count)
        } else {
            format!("{:.2}% ({} lines)", self.progress * 100.0, self.line_count)
        }))
        .block(Block::new().padding(Padding::horizontal(1)))
        .dark_gray()
        .on_black()
        .render(chunks[1], buf);

        Paragraph::new(Span::from(match self.input_mode {
            InputMode::Command => "COMMAND",
            InputMode::Viewer => "VIEWER",
            InputMode::Select => "SELECT",
        }))
        .block(Block::new().padding(Padding::horizontal(1)))
        .on_blue()
        .render(chunks[0], buf);

        Paragraph::new(self.name)
            .block(Block::new().padding(Padding::horizontal(1)))
            .alignment(Alignment::Right)
            .on_blue()
            .render(chunks[2], buf);
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
    viewer: &'a mut Viewer,
}

impl Widget for ViewerWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        self.viewer.viewport_mut().fit_view(area.height as usize);

        let view = self.viewer.update_and_view();
        let rows = view.iter().map(|(ln, data)| {
            let mut row = Row::new([Cell::from((ln + 1).to_string()), Cell::from(data.as_str())]);

            if *ln == self.viewer.viewport_mut().current() {
                row = row.on_dark_gray();
            }

            row.height(1)
        });
        // Wait til https://github.com/ratatui-org/ratatui/issues/537 is fixed
        let t = Table::new(rows).widths(&[Constraint::Percentage(5), Constraint::Percentage(95)]);

        ratatui::widgets::Widget::render(t, area, buf)
    }
}

pub struct MultiplexerWidget<'a> {
    pub mux: &'a mut Multiplexer,
    pub(super) input_mode: InputMode,
}

impl MultiplexerWidget<'_> {
    fn split_status(area: Rect) -> std::rc::Rc<[Rect]> {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(area)
    }

    fn split_tabs(area: Rect) -> std::rc::Rc<[Rect]> {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(1)])
            .split(area)
    }

    fn split_horizontal(area: Rect, len: usize) -> std::rc::Rc<[Rect]> {
        let constraints = vec![Constraint::Ratio(1, len as u32); len];
        Layout::default()
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
        let mut header =
            Paragraph::new(self.name).block(Block::new().padding(Padding::horizontal(1)));
        if self.active {
            header = header.black().on_white();
        } else {
            header = header.on_black();
        }
        header.render(area, buf);
    }
}

impl Widget for MultiplexerWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if self.mux.is_empty() {
            return Paragraph::new("empty")
                .alignment(Alignment::Center)
                .render(area, buf);
        }

        let chunks = Self::split_status(area);
        let active = self.mux.active();
        match self.mux.mode() {
            MultiplexerMode::Windows => {
                let hsplit = Self::split_horizontal(chunks[0], self.mux.len());

                for (i, (&chunk, viewer)) in
                    hsplit.into_iter().zip(self.mux.viewers_mut()).enumerate()
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
                    hsplit.into_iter().zip(self.mux.viewers_mut()).enumerate()
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

        if let Some(viewer) = self.mux.active_viewer_mut() {
            StatusWidget {
                input_mode: self.input_mode,
                progress: viewer.file().progress(),
                line_count: viewer.file().line_count(),
                name: viewer.name(),
            }
            .render(chunks[1], buf);
        }
    }
}
