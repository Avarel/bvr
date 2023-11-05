mod widgets;

use crate::ui::{
    command::{CommandApp, CursorJump, CursorMovement},
    viewer::{Multiplexer, Viewer},
};
use anyhow::Result;
use bvr::file::ShardedFile;
use crossterm::{
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event, KeyCode, KeyEventKind, KeyModifiers,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::prelude::*;
use std::{path::Path, time::Duration};

use self::widgets::{CommandWidget, MultiplexerWidget};

pub type Backend<'a> = ratatui::backend::CrosstermBackend<std::io::StdoutLock<'a>>;
pub type Terminal<'a> = ratatui::Terminal<Backend<'a>>;

#[derive(PartialEq, Clone, Copy)]
enum InputMode {
    Command,
    Viewer,
    Select,
}

/// App holds the state of the application
pub struct App {
    command: CommandApp,
    mux: Multiplexer,
    /// Current input mode
    input_mode: InputMode,
    rt: tokio::runtime::Runtime,
}

impl App {
    pub fn new(rt: tokio::runtime::Runtime) -> Self {
        Self {
            input_mode: InputMode::Viewer,
            command: CommandApp::new(),
            mux: Multiplexer::new(),
            rt,
        }
    }

    pub fn new_viewer(&mut self, path: impl AsRef<Path>) {
        let path = path.as_ref();
        let file = self.rt.block_on(tokio::fs::File::open(path)).unwrap();
        let name = path
            .file_name()
            .map(|str| str.to_string_lossy().into_owned())
            .unwrap_or_else(|| String::from("Unnamed File"));
        let viewer = Viewer::new(name, self.rt.block_on(ShardedFile::new(file, 25)).unwrap());
        self.mux.push_viewer(viewer);
    }

    pub fn run_app(&mut self, terminal: &mut Terminal) -> Result<()> {
        enable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            EnterAlternateScreen,
            EnableBracketedPaste,
            EnableMouseCapture,
        )?;

        loop {
            terminal.draw(|f| self.ui(f))?;

            if !event::poll(Duration::from_secs_f64(1.0 / 60.0))? {
                continue;
            }
            match event::read()? {
                Event::Mouse(mouse) => match mouse.kind {
                    event::MouseEventKind::ScrollDown => {
                        self.mux
                            .active_viewer_mut()
                            .map(|viewer| viewer.viewport_mut().move_view_down(2));
                    }
                    event::MouseEventKind::ScrollUp => {
                        self.mux
                            .active_viewer_mut()
                            .map(|viewer| viewer.viewport_mut().move_view_up(2));
                    }
                    _ => (),
                },
                Event::Paste(paste) => {
                    self.command.enter_str(&paste);
                }
                Event::Key(key) => match self.input_mode {
                    InputMode::Viewer => match key.code {
                        KeyCode::Char(':') => {
                            self.input_mode = InputMode::Command;
                        }
                        KeyCode::Char('i') => {
                            self.input_mode = InputMode::Select;
                        }
                        KeyCode::Esc => {
                            break;
                        }
                        KeyCode::Up => {
                            self.mux
                                .active_viewer_mut()
                                .map(|viewer| viewer.viewport_mut().move_view_up(1));
                        }
                        KeyCode::Down => {
                            self.mux
                                .active_viewer_mut()
                                .map(|viewer| viewer.viewport_mut().move_view_down(1));
                        }
                        KeyCode::Right => {
                            self.mux.move_active_right();
                        }
                        KeyCode::Left => {
                            self.mux.move_active_left();
                        }
                        _ => {}
                    },
                    InputMode::Select => match key.code {
                        KeyCode::Char(':') => {
                            self.input_mode = InputMode::Command;
                        }
                        KeyCode::Esc => {
                            self.input_mode = InputMode::Viewer;
                        }
                        KeyCode::Up => {
                            self.mux
                                .active_viewer_mut()
                                .map(|viewer| viewer.viewport_mut().move_select_up(1));
                        }
                        KeyCode::Down => {
                            self.mux
                                .active_viewer_mut()
                                .map(|viewer| viewer.viewport_mut().move_select_down(1));
                        }
                        _ => {}
                    },
                    InputMode::Command if key.kind == KeyEventKind::Press => match key.code {
                        KeyCode::Esc => {
                            self.input_mode = InputMode::Viewer;
                        }
                        KeyCode::Enter => {
                            let command = self.command.submit();
                            if command == "q" {
                                break;
                            } else if command.starts_with("open ") {
                                self.new_viewer(&command[5..]);
                            } else if command == "close" {
                                self.mux.close_active_viewer()
                            } else if command == "mux" {
                                self.mux.swap_mode();
                            }
                            self.input_mode = InputMode::Viewer;
                        }
                        KeyCode::Left => {
                            self.command.move_left(CursorMovement::new(
                                key.modifiers.contains(KeyModifiers::SHIFT),
                                if key.modifiers.contains(KeyModifiers::ALT) {
                                    CursorJump::Word
                                } else {
                                    CursorJump::None
                                },
                            ));
                        }
                        KeyCode::Right => {
                            self.command.move_right(CursorMovement::new(
                                key.modifiers.contains(KeyModifiers::SHIFT),
                                if key.modifiers.contains(KeyModifiers::ALT) {
                                    CursorJump::Word
                                } else {
                                    CursorJump::None
                                },
                            ));
                        }
                        KeyCode::Home => {
                            self.command.move_left(CursorMovement::new(
                                key.modifiers.contains(KeyModifiers::SHIFT),
                                CursorJump::Boundary,
                            ));
                        }
                        KeyCode::End => {
                            self.command.move_right(CursorMovement::new(
                                key.modifiers.contains(KeyModifiers::SHIFT),
                                CursorJump::Boundary,
                            ));
                        }
                        KeyCode::Backspace => {
                            if !self.command.delete() {
                                self.input_mode = InputMode::Viewer;
                            }
                        }
                        KeyCode::Char(to_insert) => match to_insert {
                            'b' if key.modifiers.contains(KeyModifiers::ALT) => {
                                self.command.move_left(CursorMovement::new(
                                    key.modifiers.contains(KeyModifiers::SHIFT),
                                    CursorJump::Word,
                                ));
                            }
                            'f' if key.modifiers.contains(KeyModifiers::ALT) => {
                                self.command.move_right(CursorMovement::new(
                                    key.modifiers.contains(KeyModifiers::SHIFT),
                                    CursorJump::Word,
                                ));
                            }
                            'a' if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                self.command.move_left(CursorMovement::new(
                                    key.modifiers.contains(KeyModifiers::SHIFT),
                                    CursorJump::Boundary,
                                ));
                            }
                            'e' if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                self.command.move_right(CursorMovement::new(
                                    key.modifiers.contains(KeyModifiers::SHIFT),
                                    CursorJump::Boundary,
                                ));
                            }
                            _ => self.command.enter_char(to_insert),
                        },
                        _ => {}
                    },
                    _ => {}
                },
                _ => (),
            }
        }

        // restore terminal
        disable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            DisableMouseCapture,
            DisableBracketedPaste,
            LeaveAlternateScreen,
        )?;
        terminal.show_cursor()?;

        Ok(())
    }

    fn ui(&mut self, f: &mut Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(f.size());

        f.render_widget(
            MultiplexerWidget {
                mux: &mut self.mux,
                input_mode: self.input_mode,
            },
            chunks[0],
        );

        let mut cursor = None;
        f.render_widget(
            CommandWidget {
                active: self.input_mode == InputMode::Command,
                inner: &self.command,
                cursor: &mut cursor,
            },
            chunks[1],
        );

        if let Some((x, y)) = cursor {
            f.set_cursor(x, y);
        }
    }
}
