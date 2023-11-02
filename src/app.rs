use anyhow::Result;
use bvr::file::ShardedFile;
use crossterm::event::{KeyEvent, KeyCode, KeyEventKind, KeyModifiers, MouseEventKind};
use ratatui::{prelude::*, widgets::{Paragraph, Widget, Row, Cell, Table}};
use tokio::sync::mpsc;

use crate::{command::{CommandApp, self}, tui, viewer::Viewer};

#[derive(PartialEq)]
enum InputMode {
    Command,
    Viewer,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Render,
    Resize(u16, u16),
    Quit,
}

pub struct App {
    pub should_quit: bool,
    pub should_suspend: bool,
    command: CommandApp,
    viewer: Viewer,
    input_mode: InputMode,
}

impl App {
    pub async fn new() -> Result<Self> {
        let file = tokio::fs::File::open("./Cargo.toml").await?;
        Ok(Self {
            should_quit: false,
            should_suspend: false,
            input_mode: InputMode::Viewer,
            command: CommandApp::new(),
            viewer: Viewer::new(ShardedFile::new(file, 25).await?),
        })
    }

    pub async fn run(&mut self) -> Result<()> {
        let (action_tx, mut action_rx) = mpsc::unbounded_channel();

        let stdout = std::io::stdout().lock();
        let backend = tui::Backend::new(stdout);
        let terminal = tui::Terminal::new(backend)?;
        let mut tui = tui::Tui::new(terminal)?;
        tui.enter()?;

        loop {
            if let Some(e) = tui.next().await {
                match e {
                    tui::Event::Quit => action_tx.send(Action::Quit)?,
                    tui::Event::Render => action_tx.send(Action::Render)?,
                    tui::Event::Resize(x, y) => action_tx.send(Action::Resize(x, y))?,
                    tui::Event::Mouse(mouse) => match mouse.kind {
                        MouseEventKind::ScrollUp => self.viewer.viewport_mut().move_up(),
                        MouseEventKind::ScrollDown => self.viewer.viewport_mut().move_down(),
                        _ => {}
                    },
                    tui::Event::Key(key) => {
                        match self.input_mode {
                            InputMode::Viewer => match key.code {
                                KeyCode::Char(':') => {
                                    self.input_mode = InputMode::Command;
                                }
                                KeyCode::Esc => {
                                    action_tx.send(Action::Quit)?;
                                }
                                KeyCode::Up => self.viewer.viewport_mut().move_up(),
                                KeyCode::Down => self.viewer.viewport_mut().move_down(),
                                _ => {}
                            },
                            InputMode::Command if key.kind == KeyEventKind::Press => match key.code {
                                KeyCode::Enter => {
                                    if self.command.submit() == "q" {
                                        action_tx.send(Action::Quit)?;
                                    }
                                }
                                KeyCode::Left => {
                                    self.command.move_left(command::CursorMovement::new(
                                        key.modifiers.contains(KeyModifiers::SHIFT),
                                        if key.modifiers.contains(KeyModifiers::ALT) {
                                            command::CursorJump::Word
                                        } else {
                                            command::CursorJump::None
                                        },
                                    ));
                                }
                                KeyCode::Right => {
                                    self.command.move_right(command::CursorMovement::new(
                                        key.modifiers.contains(KeyModifiers::SHIFT),
                                        if key.modifiers.contains(KeyModifiers::ALT) {
                                            command::CursorJump::Word
                                        } else {
                                            command::CursorJump::None
                                        },
                                    ));
                                }
                                KeyCode::Home => {
                                    self.command.move_left(command::CursorMovement::new(
                                        key.modifiers.contains(KeyModifiers::SHIFT),
                                        command::CursorJump::Boundary,
                                    ));
                                }
                                KeyCode::End => {
                                    self.command.move_right(command::CursorMovement::new(
                                        key.modifiers.contains(KeyModifiers::SHIFT),
                                        command::CursorJump::Boundary,
                                    ));
                                }
                                KeyCode::Char(to_insert) => match to_insert {
                                    'b' if key.modifiers.contains(KeyModifiers::ALT) => {
                                        self.command.move_left(command::CursorMovement::new(
                                            key.modifiers.contains(KeyModifiers::SHIFT),
                                            command::CursorJump::Word,
                                        ));
                                    }
                                    'f' if key.modifiers.contains(KeyModifiers::ALT) => {
                                        self.command.move_right(command::CursorMovement::new(
                                            key.modifiers.contains(KeyModifiers::SHIFT),
                                            command::CursorJump::Word,
                                        ));
                                    }
                                    'a' if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                        self.command.move_left(command::CursorMovement::new(
                                            key.modifiers.contains(KeyModifiers::SHIFT),
                                            command::CursorJump::Boundary,
                                        ));
                                    }
                                    'e' if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                        self.command.move_right(command::CursorMovement::new(
                                            key.modifiers.contains(KeyModifiers::SHIFT),
                                            command::CursorJump::Boundary,
                                        ));
                                    }
                                    _ => self.command.enter_char(to_insert),
                                },
                                KeyCode::Backspace => {
                                    if !self.command.delete() {
                                        self.input_mode = InputMode::Viewer;
                                    }
                                }
                                KeyCode::Esc => {
                                    self.input_mode = InputMode::Viewer;
                                }
                                _ => {}
                            },
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }

            while let Ok(action) = action_rx.try_recv() {
                match action {
                    Action::Quit => self.should_quit = true,
                    Action::Resize(w, h) => {
                        tui.resize(Rect::new(0, 0, w, h))?;
                        action_tx.send(Action::Render)?;
                    }
                    Action::Render => {
                        tui.draw(|f| self.ui(f))?;
                    }
                }
            }
            if self.should_quit {
                tui.stop()?;
                break;
            }
        }
        tui.exit()?;
        Ok(())
    }

    fn ui(&mut self, f: &mut Frame<'_>) {
        let overall_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(f.size());
    
        let mut cursor = None;
        f.render_widget(
            CommandWidget {
                active: self.input_mode == InputMode::Command,
                inner: &self.command,
                cursor: &mut cursor,
            },
            overall_chunks[1],
        );
    
        if let Some((x, y)) = cursor {
            f.set_cursor(x, y);
        }
    
        self.viewer
            .viewport_mut()
            .fit_view(overall_chunks[0].height as usize);
    
        let view = self.viewer.view();
        let rows = view.iter().map(|data| {
            if let Some((ln, data)) = data {
                Row::new([Cell::from((ln + 1).to_string()), Cell::from(data.as_str())])
            } else {
                Row::new([Cell::from(""), Cell::from("")])
            }
            .height(1)
        });
        let t = Table::new(rows).widths(&[Constraint::Percentage(10), Constraint::Percentage(90)]);
        f.render_widget(t, overall_chunks[0]);
    }
}

struct CommandWidget<'a> {
    inner: &'a command::CommandApp,
    cursor: &'a mut Option<(u16, u16)>,
    active: bool,
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
                command::Cursor::Singleton(_) => v.push(Span::from(self.inner.buf())),
                command::Cursor::Selection(start, end, _) => {
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
                    command::Cursor::Singleton(i) => {
                        *self.cursor = Some((command_chunks[1].x + i as u16, command_chunks[1].y));
                    }
                    command::Cursor::Selection(start, end, dir) => {
                        let x = match dir {
                            command::SelectionOrigin::Right => end,
                            command::SelectionOrigin::Left => start,
                        };
                        *self.cursor = Some((command_chunks[1].x + x as u16, command_chunks[1].y));
                    }
                }
            }
        }
        input.render(command_chunks[1], buf);
    }
}

