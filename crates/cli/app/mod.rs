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
use crate::{
    components::{
        instance::Instance,
        mux::{MultiplexerApp, MultiplexerMode},
        prompt::{self, PromptApp, PromptMovement},
        status::StatusApp,
    },
    direction::Direction, regex_compile,
};
use anyhow::Result;
use arboard::Clipboard;
use bvr_core::{buf::SegBuffer, err::Error, index::BoxedStream, matches::CompositeStrategy};
use crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::{prelude::*, widgets::Widget};
use regex::bytes::Regex;
use std::{
    borrow::Cow,
    collections::VecDeque,
    fs::OpenOptions,
    num::NonZeroUsize,
    path::{Path, PathBuf},
    time::Duration,
};

pub type Backend<'a> = ratatui::backend::CrosstermBackend<std::io::StdoutLock<'a>>;
pub type Terminal<'a> = ratatui::Terminal<Backend<'a>>;

#[derive(PartialEq, Clone, Copy)]
pub enum InputMode {
    Prompt(PromptMode),
    Normal,
    Visual,
    Filter,
}

#[derive(PartialEq, Clone, Copy)]
pub enum PromptMode {
    Command,
    Shell,
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
    clipboard: Option<Clipboard>,
    gutter: bool,
    action_queue: VecDeque<Action>,
    regex_cache: Option<(String, Option<Regex>)>,
}

impl App {
    pub fn new() -> Self {
        Self {
            mode: InputMode::Normal,
            prompt: PromptApp::new(),
            mux: MultiplexerApp::new(),
            status: StatusApp::new(),
            keybinds: Keybinding::Hardcoded,
            clipboard: Clipboard::new().ok(),
            gutter: true,
            action_queue: VecDeque::new(),
            regex_cache: None,
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

    fn enter_terminal(terminal: &mut Terminal) -> Result<()> {
        enable_raw_mode()?;
        crossterm::execute!(
            terminal.backend_mut(),
            EnterAlternateScreen,
            EnableBracketedPaste,
            EnableMouseCapture,
        )?;
        Ok(())
    }

    fn exit_terminal(terminal: &mut Terminal) -> Result<()> {
        disable_raw_mode()?;
        crossterm::execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableBracketedPaste,
            DisableMouseCapture,
        )?;
        Ok(())
    }

    pub fn run_app(&mut self, terminal: &mut Terminal) -> Result<()> {
        Self::enter_terminal(terminal)?;
        let result = self.event_loop(terminal);
        Self::exit_terminal(terminal)?;
        result
    }

    fn event_loop(&mut self, terminal: &mut Terminal) -> Result<()> {
        let mut mouse_handler = MouseHandler::new();

        loop {
            terminal.draw(|f| self.ui(f, &mut mouse_handler))?;

            let action = match self.action_queue.pop_front() {
                Some(action) => action,
                None => match mouse_handler.extract() {
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
                },
            };

            if !self.process_action(action, terminal)? {
                break;
            }
        }
        Ok(())
    }

    fn process_action(&mut self, action: Action, terminal: &mut Terminal) -> Result<bool> {
        match action {
            Action::Exit => return Ok(false),
            Action::SwitchMode(new_mode) => {
                self.prompt.take();
                self.mode = new_mode;

                if new_mode == InputMode::Visual {
                    if let Some(viewer) = self.mux.active_viewer_mut() {
                        viewer.move_selected_into_view();
                        viewer.set_follow_output(false);
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
                                if let Some(next) =
                                    viewer.compositor_mut().compute_jump(current, direction)
                                {
                                    viewer.viewport_mut().top_to(next)
                                }
                                return Ok(true);
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
                        viewer.move_select(direction, select, delta);
                        viewer.set_follow_output(false);
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
                        return Ok(true);
                    };
                    viewer.toggle_bookmark_line_number(line_number)
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
                        viewer.toggle_filter(filter_index)
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
                    let result = match self.mode {
                        InputMode::Prompt(PromptMode::Command) => {
                            let command = self.prompt.submit();
                            Ok(self.process_command(&command))
                        }
                        InputMode::Prompt(mode @ (PromptMode::NewFilter | PromptMode::NewLit)) => {
                            let command = self.prompt.take();
                            Ok(self.process_search(&command, matches!(mode, PromptMode::NewLit)))
                        }
                        InputMode::Prompt(PromptMode::Shell) => {
                            let command = self.prompt.take();
                            self.process_shell(&command, true, terminal)
                        }
                        InputMode::Normal | InputMode::Visual | InputMode::Filter => unreachable!(),
                    };
                    self.mode = InputMode::Normal;
                    return result;
                }
                CommandAction::History { direction } => {
                    if self.mode != InputMode::Prompt(PromptMode::Command) {
                        return Ok(true);
                    }
                    match direction {
                        Direction::Back => self.prompt.backward(),
                        Direction::Next => self.prompt.forward(),
                    }
                }
                CommandAction::Complete => (),
            },
            Action::ExportFile(path) => {
                if let Some(viewer) = self.mux.active_viewer_mut() {
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
        };

        Ok(true)
    }

    fn context(&mut self, s: &str) -> Result<Option<Cow<'static, str>>, std::env::VarError> {
        match s {
            "SEL" | "sel" => {
                if let Some(viewer) = self.mux.active_viewer_mut() {
                    match viewer.export_string() {
                        Ok(text) => Ok(Some(text.into())),
                        Err(err) => {
                            self.status.submit_message(
                                format!("selection expansion: {err}"),
                                Some(Duration::from_secs(2)),
                            );
                            Ok(Some("".into()))
                        }
                    }
                } else {
                    Ok(Some("".into()))
                }
            }
            s => match std::env::var(s) {
                Ok(value) => Ok(Some(value.into())),
                Err(std::env::VarError::NotPresent) => Ok(Some("".into())),
                Err(e) => Err(e),
            },
        }
    }

    fn process_shell(
        &mut self,
        command: &str,
        terminate: bool,
        terminal: &mut Terminal,
    ) -> Result<bool> {
        let Ok(expanded) = shellexpand::env_with_context(command, |s| self.context(s)) else {
            self.status.submit_message(
                "shell: expansion failed".to_string(),
                Some(Duration::from_secs(2)),
            );
            return Ok(true);
        };

        let mut shl = shlex::Shlex::new(&expanded);
        let Some(cmd) = shl.next() else {
            self.status.submit_message(
                "shell: no command provided".to_string(),
                Some(Duration::from_secs(2)),
            );
            return Ok(true);
        };

        let args = shl.by_ref().collect::<Vec<_>>();

        if shl.had_error {
            self.status.submit_message(
                "shell: lexing failed".to_string(),
                Some(Duration::from_secs(2)),
            );
            return Ok(true);
        }

        let mut command = std::process::Command::new(cmd);
        command.args(args);

        Self::exit_terminal(terminal)?;
        let mut child = match command.spawn() {
            Err(err) => {
                terminal.clear()?;
                Self::enter_terminal(terminal)?;
                self.status
                    .submit_message(format!("shell: {err}"), Some(Duration::from_secs(2)));
                return Ok(true);
            }
            Ok(child) => {
                if terminate {
                    self.mux.clear();
                }
                child
            }
        };

        let status = match child.wait() {
            Err(err) => {
                self.status
                    .submit_message(format!("shell: {err}"), Some(Duration::from_secs(2)));
                return Ok(true);
            }
            Ok(status) => status,
        };

        if terminate {
            std::process::exit(status.code().unwrap_or(0));
        }

        Ok(!terminate)
    }

    fn process_search(&mut self, pat: &str, escaped: bool) -> bool {
        if let Some(viewer) = self.mux.active_viewer_mut() {
            if let Err(err) = viewer.add_search_filter(pat, escaped) {
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
            };
        }

        true
    }

    fn process_command(&mut self, command: &str) -> bool {
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
            Some("pb" | "pbcopy") => {
                let Some(clipboard) = self.clipboard.as_mut() else {
                    self.status.submit_message(
                        "pbcopy: clipboard not available".to_string(),
                        Some(Duration::from_secs(2)),
                    );
                    return true;
                };
                if let Some(viewer) = self.mux.active_viewer_mut() {
                    match viewer.export_string() {
                        Ok(text) => match clipboard.set_text(text) {
                            Ok(_) => {
                                self.status.submit_message(
                                    "pbcopy: copied to clipboard".to_string(),
                                    Some(Duration::from_secs(2)),
                                );
                            }
                            Err(err) => {
                                self.status.submit_message(
                                    format!("pbcopy: {err}"),
                                    Some(Duration::from_secs(2)),
                                );
                            }
                        },
                        Err(err) => {
                            self.status.submit_message(
                                format!("pbcopy: {err}"),
                                Some(Duration::from_secs(2)),
                            );
                        }
                    }
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
                Some("export") => {
                    let Some(source) = self.mux.active_viewer_mut() else {
                        return true;
                    };
                    let export = source.compositor_mut().export_user_filters();

                    let Some(idx) = parts.next() else {
                        self.status.submit_message(
                            String::from("filter export: requires instance index"),
                            Some(Duration::from_secs(2)),
                        );
                        return true;
                    };

                    let Ok(idx) = idx.parse::<usize>() else {
                        self.status.submit_message(
                            format!("filter export {idx}: invalid index"),
                            Some(Duration::from_secs(2)),
                        );
                        return true;
                    };
                    let idx = idx.saturating_sub(1);
                    if self.mux.active() == idx {
                        self.status.submit_message(
                            String::from("filter export: cannot export to active instance"),
                            Some(Duration::from_secs(2)),
                        );
                        return true;
                    }
                    let Some(target) = self.mux.viewers_mut().get_mut(idx) else {
                        self.status.submit_message(
                            format!("filter export {idx}: invalid index"),
                            Some(Duration::from_secs(2)),
                        );
                        return true;
                    };
                    
                    target.import_user_filters(export);
                }
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
                        viewer.set_composite_strategy(CompositeStrategy::Union);
                    }
                }
                Some("intersect" | "i" | "&&" | "&") => {
                    if let Some(viewer) = self.mux.active_viewer_mut() {
                        viewer.set_composite_strategy(CompositeStrategy::Intersection);
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
                self.status.submit_message(
                    format!(
                        "{}: export starting (this may take a while...)",
                        path.display()
                    ),
                    Some(Duration::from_secs(2)),
                );
                self.action_queue.push_back(Action::ExportFile(path));
            }
            Some(cmd) => {
                if let Ok(line_number) = cmd.parse::<usize>() {
                    if let Some(viewer) = self.mux.active_viewer_mut() {
                        if let Some(idx) = viewer.nearest_index(line_number) {
                            viewer.viewport_mut().jump_vertically_to(idx);
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

        match self.mode {
            InputMode::Prompt(a @ (PromptMode::NewFilter | PromptMode::NewLit)) => {
                let pattern = self.prompt.buf();

                let pattern_mismatch = self
                    .regex_cache
                    .as_ref()
                    .map(|(p, _)| p != pattern)
                    .unwrap_or(true);

                if pattern_mismatch {
                    let regex = if a == PromptMode::NewLit {
                        regex_compile(&regex::escape(pattern)).ok()
                    } else {
                        regex_compile(pattern).ok()
                    };

                    self.regex_cache = Some((pattern.to_owned(), regex))
                }
            }
            InputMode::Prompt(_) | InputMode::Normal | InputMode::Visual | InputMode::Filter => {
                self.regex_cache = None;
            }
        }

        MultiplexerWidget {
            mux: &mut self.mux,
            status: &mut self.status,
            mode: self.mode,
            gutter: self.gutter,
            regex: self.regex_cache.as_ref().and_then(|(_, r)| r.as_ref()),
        }
        .render(mux_chunk, f.buffer_mut(), handler);

        let mut cursor = None;
        PromptWidget {
            mode: self.mode,
            inner: &mut self.prompt,
            cursor: &mut cursor,
        }
        .render(cmd_chunk, f.buffer_mut());

        if let Some((x, y)) = cursor {
            f.set_cursor(x, y);
        }
    }
}
