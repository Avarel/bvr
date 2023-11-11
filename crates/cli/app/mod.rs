mod actions;
mod keybinding;
mod widgets;

use crate::components::{
    command::{CommandApp, CursorMovement},
    mux::MultiplexerApp,
    status::StatusApp,
    viewer::Instance,
};
use anyhow::Result;
use bvr_core::{
    file::ShardedFile,
    index::sync::{AsyncIndex, AsyncStream},
};
use crossterm::{
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::prelude::*;
use std::{path::Path, time::Duration};

use self::{
    actions::{Action, CommandAction, ViewerAction},
    keybinding::Keybinding,
    widgets::{CommandWidget, MultiplexerWidget},
};

pub type Backend<'a> = ratatui::backend::CrosstermBackend<std::io::StdoutLock<'a>>;
pub type Terminal<'a> = ratatui::Terminal<Backend<'a>>;

#[derive(PartialEq, Clone, Copy)]
pub enum InputMode {
    Command,
    Viewer,
    Select,
}

/// App holds the state of the application
pub struct App {
    /// Current input mode
    input_mode: InputMode,
    mux: MultiplexerApp,
    status: StatusApp,
    command: CommandApp,
    keybinds: Keybinding,
    rt: tokio::runtime::Runtime,
}

impl App {
    pub fn new(rt: tokio::runtime::Runtime) -> Self {
        Self {
            input_mode: InputMode::Viewer,
            command: CommandApp::new(),
            mux: MultiplexerApp::new(),
            status: StatusApp::new(),
            keybinds: Keybinding::Default,
            rt,
        }
    }

    pub fn open_file(&mut self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        let file = self.rt.block_on(tokio::fs::File::open(path))?;
        let name = path
            .file_name()
            .map(|str| str.to_string_lossy().into_owned())
            .unwrap_or_else(|| String::from("Unnamed File"));
        Ok(self.push_instance(
            name,
            self.rt
                .block_on(ShardedFile::<AsyncIndex>::read_file(file, 25))?,
        ))
    }

    pub fn open_stream(&mut self, stream: AsyncStream) -> Result<()> {
        let name = String::from("Stream");
        Ok(self.push_instance(
            name,
            self.rt
                .block_on(ShardedFile::<AsyncIndex>::read_stream(stream))?,
        ))
    }

    fn push_instance(&mut self, name: String, file: ShardedFile<AsyncIndex>) {
        let viewer = Instance::new(name, file);
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

        self.event_loop(terminal)?;

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

    fn event_loop(&mut self, terminal: &mut Terminal) -> Result<()> {
        loop {
            terminal.draw(|f| self.ui(f))?;

            if !event::poll(Duration::from_secs_f64(1.0 / 30.0))? {
                continue;
            }

            let key = self.keybinds.map_key(self.input_mode, event::read()?);

            let Some(action) = key else { continue };

            match action {
                Action::Exit => break,
                Action::SwitchMode(new_mode) => self.input_mode = new_mode,
                Action::Viewer(action) => match action {
                    ViewerAction::Pan { direction, delta } => {
                        if let Some(viewer) = self.mux.active_viewer_mut() {
                            viewer.viewport_mut().pan_view(direction, delta as usize)
                        }
                    }
                    ViewerAction::SwitchActive(direction) => self.mux.move_active(direction),
                    ViewerAction::Move(direction) => {
                        if let Some(viewer) = self.mux.active_viewer_mut() {
                            viewer.viewport_mut().move_select(direction, 1)
                        }
                    }
                    ViewerAction::ToggleLine => {
                        if let Some(viewer) = self.mux.active_viewer_mut() {
                            let ln = viewer.viewport_mut().current();
                            viewer.mask_mut().toggle(ln);
                        }
                    }
                },
                Action::Command(action) => match action {
                    CommandAction::Move {
                        direction,
                        select,
                        jump,
                    } => self.command.move_cursor(
                        direction,
                        CursorMovement::new(
                            select,
                            match jump {
                                actions::Jump::Word => crate::components::command::CursorJump::Word,
                                actions::Jump::Boundary => {
                                    crate::components::command::CursorJump::Boundary
                                }
                                actions::Jump::None => crate::components::command::CursorJump::None,
                            },
                        ),
                    ),
                    CommandAction::Type(c) => self.command.enter_char(c),
                    CommandAction::Paste(s) => self.command.enter_str(&s),
                    CommandAction::Backspace => {
                        if !self.command.delete() {
                            self.input_mode = InputMode::Viewer;
                        }
                    }
                    CommandAction::Submit => {
                        let command = self.command.submit();
                        if command == "q" {
                            break;
                        } else if command.starts_with("open ") {
                            let path = &command[5..];
                            if let Err(err) = self.open_file(path) {
                                self.status.submit_message(
                                    format!("Error opening file `{path}`: {err}"),
                                    Some(Duration::from_secs(2)),
                                );
                            }
                        } else if command == "close" {
                            self.mux.close_active_viewer()
                        } else if command == "mux" {
                            self.mux.swap_mode();
                        } else if command == "clearmask" {
                            if let Some(viewer) = self.mux.active_viewer_mut() {
                                viewer.clear_mask()
                            }
                        } else if command == "e" {
                            if let Some(viewer) = self.mux.active_viewer_mut() {
                                viewer
                                    .viewport_mut()
                                    .pan_view(crate::direction::VDirection::Down, usize::MAX);
                            }
                        } else {
                            self.status.submit_message(
                                format!("Invalid command `{command}`"),
                                Some(Duration::from_secs(2)),
                            );
                        }
                        self.input_mode = InputMode::Viewer;
                    }
                },
            }
        }
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
                status: &mut self.status,
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
