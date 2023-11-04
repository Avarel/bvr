use crate::ui::command::{CommandApp, Cursor, SelectionOrigin};
use ratatui::{prelude::*, widgets::*};

use super::InputMode;

pub struct StatusWidget {
    pub(super) input_mode: InputMode,
    pub progress: f64,
    pub line_count: usize,
}

impl Widget for StatusWidget {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let command_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(1), Constraint::Length(9)])
            .split(area);

        Paragraph::new(Span::from(if self.progress > 1.0 {
            format!("100% ({} lines)", self.line_count)
        } else {
            format!("{:.2}% ({} lines)", self.progress * 100.0, self.line_count)
        }))
        .dark_gray()
        .on_black()
        .render(command_chunks[0], buf);

        Paragraph::new(Span::from(match self.input_mode {
            InputMode::Command => "COMMAND",
            InputMode::Viewer => "VIEWER",
            InputMode::Select => "SELECT",
        }))
        .alignment(Alignment::Center)
        .on_blue()
        .render(command_chunks[1], buf);
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
