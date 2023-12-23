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
use bvr_core::{buf::SegBuffer, index::inflight::Stream, InflightIndex};
use crossterm::{
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::prelude::*;
use regex::bytes::RegexBuilder;
use std::{num::NonZeroUsize, path::Path, time::Duration};

use self::{
    actions::{Action, CommandAction, Delta, ViewerAction},
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
    Filter,
}

pub struct App {
    mode: InputMode,
    mux: MultiplexerApp,
    status: StatusApp,
    command: CommandApp,
    keybinds: Keybinding,
}

impl App {
    pub fn new() -> Self {
        Self {
            mode: InputMode::Viewer,
            command: CommandApp::new(),
            mux: MultiplexerApp::new(),
            status: StatusApp::new(),
            keybinds: Keybinding::Hardcoded,
        }
    }

    pub fn open_file(&mut self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        let file = std::fs::File::open(path)?;
        let name = path
            .file_name()
            .map(|str| str.to_string_lossy().into_owned())
            .unwrap_or_else(|| String::from("Unnamed File"));
        self.push_instance(
            name,
            SegBuffer::<InflightIndex>::read_file(file, NonZeroUsize::new(25).unwrap())?,
        );
        Ok(())
    }

    pub fn open_stream(&mut self, stream: Stream) -> Result<()> {
        let name = String::from("Stream");
        self.push_instance(name, SegBuffer::<InflightIndex>::read_stream(stream));
        Ok(())
    }

    fn push_instance(&mut self, name: String, file: SegBuffer<InflightIndex>) {
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

            let key = self.keybinds.map_key(self.mode, event::read()?);

            let Some(action) = key else { continue };

            match action {
                Action::Exit => break,
                Action::SwitchMode(new_mode) => {
                    self.command.submit();
                    self.mode = new_mode;

                    if new_mode == InputMode::Select {
                        if let Some(viewer) = self.mux.active_viewer_mut() {
                            viewer.viewport_mut().move_select_within_view();
                        }
                    }
                }
                Action::Viewer(action) => match action {
                    ViewerAction::Pan { direction, delta } => {
                        if let Some(viewer) = self.mux.active_viewer_mut() {
                            let delta = match delta {
                                Delta::Number(n) => usize::from(n),
                                Delta::Page => viewer.viewport().height(),
                                Delta::HalfPage => viewer.viewport().height().div_ceil(2),
                                Delta::Boundary => usize::MAX,
                            };
                            viewer.viewport_mut().pan_view(direction, delta);
                        }
                    }
                    ViewerAction::SwitchActive(direction) => self.mux.move_active(direction),
                    ViewerAction::Move { direction, delta } => {
                        if let Some(viewer) = self.mux.active_viewer_mut() {
                            let delta = match delta {
                                Delta::Number(n) => usize::from(n),
                                Delta::Page => viewer.viewport().height(),
                                Delta::HalfPage => viewer.viewport().height().div_ceil(2),
                                Delta::Boundary => usize::MAX,
                            };
                            viewer.viewport_mut().move_select(direction, delta)
                        }
                    }
                    ViewerAction::ToggleLine => {
                        if let Some(viewer) = self.mux.active_viewer_mut() {
                            let ln = viewer.current_selected_file_line();
                            viewer.filterer.filters.bookmarks_mut().toggle(ln);
                        }
                    }
                },
                Action::Filter(action) => match action {
                    actions::FilterAction::Move { direction, delta } => {
                        if let Some(viewer) = self.mux.active_viewer_mut() {
                            let viewport = &mut viewer.filterer.viewport;
                            let delta = match delta {
                                Delta::Number(n) => usize::from(n),
                                Delta::Page => viewport.height(),
                                Delta::HalfPage => viewport.height().div_ceil(2),
                                Delta::Boundary => usize::MAX,
                            };
                            viewport.move_select(direction, delta)
                        }
                    }
                    actions::FilterAction::Toggle => {
                        if let Some(viewer) = self.mux.active_viewer_mut() {
                            viewer.filterer.current_filter_mut().toggle();
                            viewer.filterer.compute_composite();
                        }
                    },
                    actions::FilterAction::Remove => {
                        if let Some(viewer) = self.mux.active_viewer_mut() {
                            viewer.filterer.remove_current_filter();
                            viewer.filterer.compute_composite();
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
                            self.mode = InputMode::Viewer;
                        }
                    }
                    CommandAction::Submit => {
                        let command = self.command.submit();
                        if !self.process_command(command) {
                            break;
                        }
                        self.mode = InputMode::Viewer;
                    }
                },
            }
        }
        Ok(())
    }

    fn process_command(&mut self, command: String) -> bool {
        if command == "q" {
            return false;
        } else if let Some(path) = command.strip_prefix("open ") {
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
        } else if command == "clearfilter" {
            if let Some(viewer) = self.mux.active_viewer_mut() {
                viewer.filterer.filters.clear();
                viewer.filterer.compute_composite();
            }
        } else if let Some(pat) = command.strip_prefix("find ") {
            let regex = match RegexBuilder::new(pat).case_insensitive(true).build() {
                Ok(r) => r,
                Err(err) => {
                    self.status.submit_message(
                        format!("Invalid regex `{pat}`: {err}"),
                        Some(Duration::from_secs(2)),
                    );
                    return true;
                }
            };

            if let Some(viewer) = self.mux.active_viewer_mut() {
                viewer.filter_search(regex);
                viewer.filterer.compute_composite();
            }
        } else if let Some(pat) = command.strip_prefix("findl ") {
            let pat = regex::escape(pat);
            let regex = match RegexBuilder::new(&pat).case_insensitive(true).build() {
                Ok(r) => r,
                Err(err) => {
                    self.status.submit_message(
                        format!("Invalid regex `{pat}`: {err}"),
                        Some(Duration::from_secs(2)),
                    );
                    return true;
                }
            };

            if let Some(viewer) = self.mux.active_viewer_mut() {
                viewer.filter_search(regex);
                viewer.filterer.compute_composite();
            }
        } else {
            self.status.submit_message(
                format!("Invalid command `{command}`"),
                Some(Duration::from_secs(2)),
            );
        }
        true
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
                mode: self.mode,
            },
            chunks[0],
        );

        let mut cursor = None;
        f.render_widget(
            CommandWidget {
                active: self.mode == InputMode::Command,
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
