mod actions;
mod keybinding;
mod mouse;
mod widgets;

use self::{
    actions::{Action, CommandAction, NormalAction, VisualAction},
    keybinding::Keybinding,
    mouse::MouseHandler,
    widgets::{MultiplexerWidget, PromptWidget},
};
use crate::components::{
    filters::Filter,
    instance::Instance,
    mux::{MultiplexerApp, MultiplexerMode},
    prompt::{self, PromptApp, PromptMovement},
    status::StatusApp,
};
use anyhow::Result;
use bvr_core::{buf::SegBuffer, err::Error, index::BoxedStream, matches::CompositeStrategy};
use crossterm::{
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{prelude::*, widgets::Widget};
use regex::bytes::RegexBuilder;
use std::{
    borrow::Cow,
    fs::OpenOptions,
    num::NonZeroUsize,
    path::{Path, PathBuf},
    time::Duration,
};

pub type Backend<'a> = ratatui::backend::CrosstermBackend<std::io::StdoutLock<'a>>;
pub type Terminal<'a> = ratatui::Terminal<Backend<'a>>;

#[derive(PartialEq, Clone, Copy)]
pub enum InputMode {
    Command(PromptMode),
    Normal,
    Visual,
    Filter,
}

#[derive(PartialEq, Clone, Copy)]
pub enum PromptMode {
    Command,
    NewFilter,
    NewLit,
}

pub enum ViewDelta {
    Number(u16),
    Page,
    HalfPage,
    Boundary,
    Match,
}

pub struct App {
    mode: InputMode,
    mux: MultiplexerApp,
    status: StatusApp,
    prompt: PromptApp,
    keybinds: Keybinding,
    gutter: bool,
}

impl App {
    pub fn new() -> Self {
        Self {
            mode: InputMode::Normal,
            prompt: PromptApp::new(),
            mux: MultiplexerApp::new(),
            status: StatusApp::new(),
            keybinds: Keybinding::Hardcoded,
            gutter: true,
        }
    }

    pub fn open_file(&mut self, path: &Path) -> Result<()> {
        let file = std::fs::File::open(path)?;
        let name = path
            .file_name()
            .map(|str| str.to_string_lossy().into_owned())
            .unwrap_or_else(|| String::from("Unnamed File"));
        self.push_instance(
            name,
            SegBuffer::read_file(file, NonZeroUsize::new(25).unwrap(), false)?,
        );
        Ok(())
    }

    pub fn open_stream(&mut self, name: String, stream: BoxedStream) -> Result<()> {
        self.push_instance(name, SegBuffer::read_stream(stream, false)?);
        Ok(())
    }

    fn push_instance(&mut self, name: String, file: SegBuffer) {
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
        let mut mouse_handler = MouseHandler::new();

        loop {
            terminal.draw(|f| self.ui(f, &mut mouse_handler))?;

            let action = match mouse_handler.extract() {
                Some(action) => action,
                None => {
                    if !event::poll(Duration::from_secs_f64(1.0 / 30.0))? {
                        continue;
                    }

                    let mut event = event::read()?;
                    let key = self.keybinds.map_key(self.mode, &mut event);
                    mouse_handler.publish_event(event);
                    let Some(action) = key else { continue };
                    action
                }
            };

            if !self.process_action(action) {
                break;
            }
        }
        Ok(())
    }

    fn process_action(&mut self, action: Action) -> bool {
        match action {
            Action::Exit => return false,
            Action::SwitchMode(new_mode) => {
                self.prompt.submit();
                self.mode = new_mode;

                if new_mode == InputMode::Visual {
                    if let Some(viewer) = self.mux.active_viewer_mut() {
                        viewer.move_selected_into_view()
                    }
                }
            }
            Action::Normal(action) => match action {
                NormalAction::PanVertical {
                    direction,
                    delta,
                    target_view,
                } => {
                    let viewer = if let Some(index) = target_view {
                        self.mux.viewers_mut().get_mut(index)
                    } else {
                        self.mux.active_viewer_mut()
                    };

                    if let Some(viewer) = viewer {
                        let delta = match delta {
                            ViewDelta::Number(n) => usize::from(n),
                            ViewDelta::Page => viewer.viewport().height(),
                            ViewDelta::HalfPage => viewer.viewport().height().div_ceil(2),
                            ViewDelta::Boundary => usize::MAX,
                            ViewDelta::Match => {
                                let current = viewer.viewport().top();
                                if let Some(next) = viewer.compositor_mut().compute_jump(current, direction) {
                                    viewer.viewport_mut().top_to(next)
                                }
                                return true;
                            }
                        };
                        viewer.viewport_mut().pan_vertical(direction, delta);
                        viewer.set_follow_output(false);
                    }
                }
                NormalAction::PanHorizontal {
                    direction,
                    delta,
                    target_view,
                } => {
                    let viewer = if let Some(index) = target_view {
                        self.mux.viewers_mut().get_mut(index)
                    } else {
                        self.mux.active_viewer_mut()
                    };

                    if let Some(viewer) = viewer {
                        let delta = match delta {
                            ViewDelta::Number(n) => usize::from(n),
                            ViewDelta::Page => viewer.viewport().width(),
                            ViewDelta::HalfPage => viewer.viewport().width().div_ceil(2),
                            _ => 0,
                        };
                        viewer.viewport_mut().pan_horizontal(direction, delta);
                        viewer.set_follow_output(false);
                    }
                }
                NormalAction::FollowOutput => {
                    if let Some(viewer) = self.mux.active_viewer_mut() {
                        viewer.set_follow_output(true);
                    }
                }
                NormalAction::SwitchActiveIndex { target_view } => {
                    self.mux.move_active_index(target_view)
                }
                NormalAction::SwitchActive(direction) => self.mux.move_active(direction),
            },
            Action::Visual(action) => match action {
                VisualAction::Move {
                    direction,
                    select,
                    delta,
                } => {
                    if let Some(viewer) = self.mux.active_viewer_mut() {
                        viewer.move_select(direction, select, delta)
                    }
                }
                VisualAction::ToggleSelectedLine => {
                    if let Some(viewer) = self.mux.active_viewer_mut() {
                        viewer.toggle_select_bookmarks();
                    }
                }
                VisualAction::ToggleLine {
                    target_view,
                    line_number,
                } => {
                    let Some(viewer) = self.mux.viewers_mut().get_mut(target_view) else {
                        return true;
                    };
                    viewer
                        .compositor_mut()
                        .filters_mut()
                        .bookmarks_mut()
                        .toggle(line_number);
                    // TODO: future optimization opportunity, do not need to recompute composite
                    // if the line number is part of a search filter and bookmark is not the only
                    // filter that satisfies the line number
                    viewer.invalidate_cache();
                }
            },
            Action::Filter(action) => match action {
                actions::FilterAction::Move {
                    direction,
                    select,
                    delta,
                } => {
                    if let Some(viewer) = self.mux.active_viewer_mut() {
                        viewer
                            .compositor_mut()
                            .move_select(direction, select, delta)
                    }
                }
                actions::FilterAction::ToggleSelectedFilter => {
                    if let Some(viewer) = self.mux.active_viewer_mut() {
                        viewer.toggle_select_filters();
                    }
                }
                actions::FilterAction::RemoveSelectedFilter => {
                    if let Some(viewer) = self.mux.active_viewer_mut() {
                        viewer.remove_select_filter();
                    }
                }
                actions::FilterAction::ToggleFilter {
                    target_view,
                    filter_index,
                } => {
                    if let Some(viewer) = self.mux.viewers_mut().get_mut(target_view) {
                        viewer
                            .compositor_mut()
                            .filters_mut()
                            .get_mut(filter_index)
                            .map(Filter::toggle);
                        viewer.invalidate_cache();
                    }
                }
            },
            Action::Command(action) => match action {
                CommandAction::Move {
                    direction,
                    select,
                    jump,
                } => self.prompt.move_cursor(
                    direction,
                    PromptMovement::new(
                        select,
                        match jump {
                            actions::CommandJump::Word => prompt::PromptDelta::Word,
                            actions::CommandJump::Boundary => prompt::PromptDelta::Boundary,
                            actions::CommandJump::None => prompt::PromptDelta::Number(1),
                        },
                    ),
                ),
                CommandAction::Type(c) => self.prompt.enter_char(c),
                CommandAction::Paste(s) => self.prompt.enter_str(&s),
                CommandAction::Backspace => {
                    if !self.prompt.delete() {
                        self.mode = InputMode::Normal;
                    }
                }
                CommandAction::Submit => {
                    let command = self.prompt.submit();
                    let result = match self.mode {
                        InputMode::Command(PromptMode::Command) => self.process_command(command),
                        InputMode::Command(PromptMode::NewFilter) => {
                            self.process_search(&command, false)
                        }
                        InputMode::Command(PromptMode::NewLit) => {
                            self.process_search(&command, true)
                        }
                        InputMode::Normal | InputMode::Visual | InputMode::Filter => unreachable!(),
                    };
                    self.mode = InputMode::Normal;
                    return result;
                }
                CommandAction::Complete => (),
            },
        };

        true
    }

    fn process_search(&mut self, pat: &str, escaped: bool) -> bool {
        let pat = if escaped {
            Cow::Owned(regex::escape(pat))
        } else {
            Cow::Borrowed(pat)
        };
        let regex = match RegexBuilder::new(&pat).case_insensitive(true).build() {
            Ok(r) => r,
            Err(err) => {
                self.status.submit_message(
                    match err {
                        regex::Error::Syntax(err) => format!("{pat}: syntax ({err})"),
                        regex::Error::CompiledTooBig(sz) => {
                            format!("{pat}: regex surpassed size limit ({sz} bytes)")
                        }
                        _ => format!("{pat}: {err}"),
                    },
                    Some(Duration::from_secs(2)),
                );
                return true;
            }
        };

        if let Some(viewer) = self.mux.active_viewer_mut() {
            viewer.filter_search(regex);
        }

        true
    }

    fn process_command(&mut self, command: String) -> bool {
        let mut parts = command.split_whitespace();

        match parts.next() {
            Some("quit" | "q") => return false,
            Some("open" | "o") => {
                let path = parts.collect::<PathBuf>();
                if let Err(err) = self.open_file(path.as_ref()) {
                    self.status.submit_message(
                        format!("{}: {err}", path.display()),
                        Some(Duration::from_secs(2)),
                    );
                }
            }
            Some("close" | "c") => {
                if self.mux.active_viewer_mut().is_some() {
                    self.mux.close_active_viewer()
                } else {
                    self.status.submit_message(
                        String::from("No active viewer"),
                        Some(Duration::from_secs(2)),
                    );
                }
            }
            Some("gutter" | "g") => {
                self.gutter = !self.gutter;
            }
            Some("mux" | "m") => match parts.next() {
                Some("tabs" | "t" | "none") => self.mux.set_mode(MultiplexerMode::Tabs),
                Some("split" | "s" | "win") => self.mux.set_mode(MultiplexerMode::Panes),
                Some(style) => {
                    self.status.submit_message(
                        format!("mux {style}: invalid style, one of `tabs`, `split`"),
                        Some(Duration::from_secs(2)),
                    );
                }
                None => self.mux.set_mode(self.mux.mode().swap()),
            },
            Some("filter" | "find" | "f") => match parts.next() {
                Some("regex" | "r") => {
                    let pat = parts.collect::<String>();
                    return self.process_search(&pat, false);
                }
                Some("literal" | "lit" | "l") => {
                    let pat = parts.collect::<String>();
                    return self.process_search(&pat, true);
                }
                Some("clear" | "c") => {
                    if let Some(viewer) = self.mux.active_viewer_mut() {
                        viewer.compositor_mut().filters_mut().clear();
                    }
                }
                Some("union" | "u" | "||" | "|") => {
                    if let Some(viewer) = self.mux.active_viewer_mut() {
                        viewer
                            .compositor_mut()
                            .set_strategy(CompositeStrategy::Union);
                        viewer.invalidate_cache();
                    }
                }
                Some("intersect" | "i" | "&&" | "&") => {
                    if let Some(viewer) = self.mux.active_viewer_mut() {
                        viewer
                            .compositor_mut()
                            .set_strategy(CompositeStrategy::Intersection);
                        viewer.invalidate_cache();
                    }
                }
                Some(cmd) => {
                    self.status.submit_message(
                        format!("filter {cmd}: invalid subcommand"),
                        Some(Duration::from_secs(2)),
                    );
                }
                None => {
                    self.status.submit_message(
                        String::from("filter: requires subcommand, one of `r[egex]`, `l[it]`, `clear`, `union`, `intersect`"),
                        Some(Duration::from_secs(2)),
                    );
                }
            },
            Some("export") => {
                let path = parts.collect::<PathBuf>();
                if let Some(viewer) = self.mux.active_viewer_mut() {
                    self.status.submit_message(
                        format!(
                            "{}: export starting (this may take a while...)",
                            path.display()
                        ),
                        Some(Duration::from_secs(2)),
                    );
                    if let Err(err) = OpenOptions::new()
                        .create_new(true)
                        .write(true)
                        .truncate(true)
                        .open(&path)
                        .map_err(Error::from)
                        .and_then(|file| viewer.export_file(file))
                    {
                        self.status.submit_message(
                            format!("{}: {err}", path.display()),
                            Some(Duration::from_secs(2)),
                        );
                    } else {
                        self.status.submit_message(
                            format!("{}: export complete", path.display()),
                            Some(Duration::from_secs(2)),
                        );
                    }
                } else {
                    self.status.submit_message(
                        String::from("No active viewer"),
                        Some(Duration::from_secs(2)),
                    );
                }
            }
            Some(cmd) => {
                if let Ok(line_number) = cmd.parse::<usize>() {
                    if let Some(viewer) = self.mux.active_viewer_mut() {
                        if let Some(idx) = viewer.nearest_index(line_number) {
                            viewer.viewport_mut().jump_to(idx);
                        }
                    }
                } else {
                    self.status.submit_message(
                        format!("{cmd}: Invalid command"),
                        Some(Duration::from_secs(2)),
                    )
                }
            }
            None => return true,
        }

        true
    }

    fn ui(&mut self, f: &mut Frame, handler: &mut MouseHandler) {
        let [mux_chunk, cmd_chunk] = MultiplexerWidget::split_bottom(f.size(), 1);

        MultiplexerWidget {
            mux: &mut self.mux,
            status: &mut self.status,
            mode: self.mode,
            gutter: self.gutter,
        }
        .render(mux_chunk, f.buffer_mut(), handler);

        let mut cursor = None;
        PromptWidget {
            mode: self.mode,
            inner: &self.prompt,
            cursor: &mut cursor,
        }
        .render(cmd_chunk, f.buffer_mut());

        if let Some((x, y)) = cursor {
            f.set_cursor(x, y);
        }
    }
}
