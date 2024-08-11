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
        config::filter::FilterData,
        instance::Instance,
        mux::{MultiplexerApp, MultiplexerMode},
        prompt::{self, PromptApp, PromptMovement},
        status::StatusApp,
    },
    direction::Direction,
    regex_compile,
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
use regex::bytes::Regex;
use serde::{Deserialize, Serialize};
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

#[derive(PartialEq, Clone, Copy, Serialize, Deserialize)]
#[serde(tag = "mode")]
pub enum InputMode {
    Prompt(PromptMode),
    Normal,
    Visual,
    Filter,
}

impl InputMode {
    pub fn is_prompt_search(&self) -> bool {
        matches!(self, InputMode::Prompt(PromptMode::Search { .. }))
    }
}

#[derive(PartialEq, Clone, Copy, Serialize, Deserialize)]
#[serde(tag = "prompt")]
pub enum PromptMode {
    Command,
    Shell { pipe: bool },
    Search { escaped: bool },
}

#[derive(Serialize, Deserialize, Clone, Copy)]
#[serde(tag = "delta")]
pub enum ViewDelta {
    Number(u16),
    Page,
    HalfPage,
    Boundary,
    Match,
}

struct RegexCache {
    pattern: String,
    escaped: bool,
    regex: Option<Regex>,
}

pub struct App<'term> {
    term: Terminal<'term>,
    mode: InputMode,
    mux: MultiplexerApp,
    status: StatusApp,
    prompt: PromptApp,
    keybinds: Keybinding,
    clipboard: Option<Clipboard>,
    filter_data: FilterData,
    gutter: bool,
    action_queue: VecDeque<Action>,
    regex_cache: Option<RegexCache>,
    mouse_capture: bool,
    linked_filters: bool,
}

impl Drop for App<'_> {
    fn drop(&mut self) {
        self.exit_terminal()
            .expect("exiting terminal should not error")
    }
}

impl<'term> App<'term> {
    pub fn new(term: Terminal<'term>) -> Self {
        Self {
            term,
            mode: InputMode::Normal,
            prompt: PromptApp::new(),
            mux: MultiplexerApp::new(),
            status: StatusApp::new(),
            filter_data: FilterData::new(),
            keybinds: Keybinding::Hardcoded,
            clipboard: Clipboard::new().ok(),
            gutter: true,
            action_queue: VecDeque::new(),
            regex_cache: None,
            mouse_capture: true,
            linked_filters: false,
        }
    }

    pub fn open_file(&mut self, path: &Path) -> Result<()> {
        let load_filters = self.mux.is_empty() && self.filter_data.is_persistent().unwrap_or(false);

        let file = std::fs::File::open(path)?;
        let name = path
            .file_name()
            .map(|str| str.to_string_lossy().into_owned())
            .unwrap_or_else(|| String::from("Unnamed File"));
        self.push_instance(
            name,
            SegBuffer::read_file(file, NonZeroUsize::new(25).unwrap(), false)?,
        );

        if load_filters {
            let filter_sets = match self.filter_data.filters() {
                Ok(filters) => filters,
                Err(err) => {
                    self.status.msg(format!("filter persist/load: {err}"));
                    return Ok(());
                }
            };
            let viewer = self.mux.active_viewer_mut().unwrap();
            match filter_sets.first() {
                Some(export) => viewer.import_user_filters(export),
                None => {}
            }
        }
        if self.linked_filters {
            if let Some(source) = self.mux.active_viewer_mut() {
                let export = source.compositor_mut().export_user_filters();
                let cursor = *source.compositor_mut().cursor();
                
                let viewer = self.mux.viewers_mut().last_mut().unwrap();
                viewer.import_user_filters(&export);
                viewer.compositor_mut().set_cursor(cursor)
            }
        }

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

    fn enter_terminal(&mut self) -> Result<()> {
        enable_raw_mode()?;
        crossterm::execute!(
            self.term.backend_mut(),
            EnterAlternateScreen,
            EnableBracketedPaste,
            EnableMouseCapture,
        )?;
        Ok(())
    }

    fn exit_terminal(&mut self) -> Result<()> {
        disable_raw_mode()?;
        if self.mouse_capture {
            crossterm::execute!(
                self.term.backend_mut(),
                DisableMouseCapture,
                DisableBracketedPaste,
                LeaveAlternateScreen,
            )?;
        } else {
            crossterm::execute!(
                self.term.backend_mut(),
                DisableBracketedPaste,
                LeaveAlternateScreen,
            )?;
        }
        Ok(())
    }

    fn toggle_mouse_capture(&mut self) -> Result<()> {
        self.mouse_capture = !self.mouse_capture;
        if self.mouse_capture {
            crossterm::execute!(self.term.backend_mut(), EnableMouseCapture)?;
        } else {
            crossterm::execute!(self.term.backend_mut(), DisableMouseCapture)?;
        }
        Ok(())
    }

    pub fn run_app(mut self) -> Result<()> {
        self.enter_terminal()?;
        let result = self.event_loop();

        if self.filter_data.is_persistent().unwrap_or(false) {
            if let Some(source) = self.mux.active_viewer_mut() {
                let export = source.compositor_mut().export_user_filters();

                if let Err(err) = self.filter_data.add_filter(export) {
                    self.status.msg(format!("filter save: {err}"));
                }

                self.status.msg("filter save: saved filters".to_string());
            }
        }

        result
    }

    fn event_loop(&mut self) -> Result<()> {
        let mut mouse_handler = MouseHandler::new();

        loop {
            let cursor = self.ui(&mut mouse_handler);
            self.term.draw(|f| {
                if let Some(cursor) = cursor {
                    f.set_cursor_position(cursor);
                }
            })?;

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

            if !self.process_action(action)? {
                break;
            }
        }
        Ok(())
    }

    fn get_target_view(&mut self, target_view: Option<usize>) -> Option<&mut Instance> {
        if let Some(index) = target_view {
            self.mux.viewers_mut().get_mut(index)
        } else {
            self.mux.active_viewer_mut()
        }
    }

    fn process_filter_action<F>(&mut self, mut f: F)
    where
        F: FnMut(&mut Instance),
    {
        if self.linked_filters {
            self.mux.viewers_mut().iter_mut().for_each(f);
        } else if let Some(viewer) = self.mux.active_viewer_mut() {
            f(viewer);
        }
    }

    fn process_action(&mut self, action: Action) -> Result<bool> {
        match action {
            Action::Exit => return Ok(false),
            Action::SwitchMode(new_mode) => {
                if !self.mode.is_prompt_search() || !new_mode.is_prompt_search() {
                    self.prompt.take();
                }
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
                    if let Some(viewer) = self.get_target_view(target_view) {
                        viewer.move_viewport_vertical(direction, delta)
                    }
                }
                NormalAction::PanHorizontal {
                    direction,
                    delta,
                    target_view,
                } => {
                    if let Some(viewer) = self.get_target_view(target_view) {
                        viewer.move_viewport_horizontal(direction, delta)
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
                    if let Some(viewer) = self.mux.viewers_mut().get_mut(target_view) {
                        viewer.toggle_bookmark_line_number(line_number)
                    }
                }
            },
            Action::Filter(action) => match action {
                actions::FilterAction::Move {
                    direction,
                    select,
                    delta,
                } => {
                    self.process_filter_action(|viewer| {
                        viewer
                            .compositor_mut()
                            .move_select(direction, select, delta)
                    });
                }
                actions::FilterAction::ToggleSelectedFilter => {
                    self.process_filter_action(|viewer| {
                        let selected_filters = viewer.selected_filters();
                        viewer.toggle_filters(selected_filters);
                    });
                }
                actions::FilterAction::RemoveSelectedFilter => {
                    self.process_filter_action(|viewer| {
                        let selected_filters = viewer.selected_filters();
                        viewer.remove_filters(selected_filters);
                    });
                }
                actions::FilterAction::ToggleFilter {
                    target_view,
                    filter_index,
                } => {
                    if self.linked_filters {
                        // TODO: handle this
                        return Ok(true);
                    }
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
                CommandAction::Type { input } => self.prompt.enter_char(input),
                CommandAction::Paste { input } => self.prompt.enter_str(&input),
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
                        InputMode::Prompt(PromptMode::Search { escaped }) => {
                            let command = self.prompt.take();
                            Ok(self.process_search(&command, escaped))
                        }
                        InputMode::Prompt(PromptMode::Shell { pipe }) => {
                            let command = self.prompt.take();
                            self.process_shell(&command, true, pipe)
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
                        .and_then(|mut file| viewer.write_bytes(&mut file))
                    {
                        self.status.msg(format!("{}: {err}", path.display()));
                    } else {
                        self.status
                            .msg(format!("{}: export complete", path.display()));
                    }
                } else {
                    self.status.msg(String::from("No active viewer"));
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
                            self.status.msg(format!("selection expansion: {err}"));
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

    fn replicate_filters_on_all_viewers(&mut self) {
        if let Some(source) = self.mux.active_viewer_mut() {
            let export = source.compositor_mut().export_user_filters();
            let cursor = *source.compositor_mut().cursor();
            let active = self.mux.active();
            self.mux
                .viewers_mut()
                .iter_mut()
                .enumerate()
                .filter(|(i, _)| *i != active)
                .for_each(|(_, viewer)| {
                    viewer.import_user_filters(&export);
                    viewer.compositor_mut().set_cursor(cursor)
                });
        }
    }

    fn process_shell(&mut self, command: &str, terminate: bool, pipe: bool) -> Result<bool> {
        let Ok(expanded) = shellexpand::env_with_context(command, |s| self.context(s)) else {
            self.status.msg("shell: expansion failed".to_string());
            return Ok(true);
        };

        let mut shl = shlex::Shlex::new(&expanded);
        let Some(cmd) = shl.next() else {
            self.status.msg("shell: no command provided".to_string());
            return Ok(true);
        };

        let args = shl.by_ref().collect::<Vec<_>>();

        if shl.had_error {
            self.status.msg("shell: lexing failed".to_string());
            return Ok(true);
        }

        let mut command = std::process::Command::new(cmd);
        command.args(args);

        self.exit_terminal()?;
        let mut child = match command.spawn() {
            Err(err) => {
                self.term.clear()?;
                self.enter_terminal()?;
                self.status.msg(format!("shell: {err}"));
                return Ok(true);
            }
            Ok(child) => child,
        };

        if pipe {
            let mut stdin = child.stdin.take().unwrap();
            if let Some(viewer) = self.mux.active_viewer_mut() {
                viewer.write_bytes(&mut stdin)?;
            }
        }

        if terminate {
            self.mux.clear();
        }

        let status = match child.wait() {
            Err(err) => {
                self.status.msg(format!("shell: {err}"));
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
        let mut e = None;
        self.process_filter_action(|viewer: &mut Instance| {
            if let Err(err) = viewer.add_search_filter(pat, escaped) {
                e.get_or_insert(err);
            };
        });

        if let Some(err) = e {
            self.status.msg(match err {
                regex::Error::Syntax(err) => format!("{pat}: syntax ({err})"),
                regex::Error::CompiledTooBig(sz) => {
                    format!("{pat}: regex surpassed size limit ({sz} bytes)")
                }
                _ => format!("{pat}: {err}"),
            });
        }

        true
    }

    fn process_command(&mut self, command: &str) -> bool {
        let mut parts = command.split_whitespace();

        match parts.next() {
            Some("quit" | "q") => return false,
            Some("mcap") => {
                if let Err(_) = self.toggle_mouse_capture() {
                    self.status.msg("mouse capture toggle failed".to_string());
                }
                return true;
            }
            Some("open" | "o") => {
                let path = parts.collect::<PathBuf>();
                if let Err(err) = self.open_file(path.as_ref()) {
                    self.status.msg(format!("{}: {err}", path.display()));
                }
            }
            Some("pb" | "pbcopy") => {
                let Some(clipboard) = self.clipboard.as_mut() else {
                    self.status
                        .msg("pbcopy: clipboard not available".to_string());
                    return true;
                };
                if let Some(viewer) = self.mux.active_viewer_mut() {
                    match viewer.export_string() {
                        Ok(text) => match clipboard.set_text(text) {
                            Ok(_) => {
                                self.status.msg("pbcopy: copied to clipboard".to_string());
                            }
                            Err(err) => {
                                self.status.msg(format!("pbcopy: {err}"));
                            }
                        },
                        Err(err) => {
                            self.status.msg(format!("pbcopy: {err}"));
                        }
                    }
                }
            }
            Some("close" | "c") => {
                if self.mux.active_viewer_mut().is_some() {
                    self.mux.close_active_viewer()
                } else {
                    self.status.msg(String::from("No active viewer"));
                }
            }
            Some("gutter" | "g") => {
                self.gutter = !self.gutter;
            }
            Some("mux" | "m") => match parts.next() {
                Some("tabs" | "t" | "none") => self.mux.set_mode(MultiplexerMode::Tabs),
                Some("split" | "s" | "win") => self.mux.set_mode(MultiplexerMode::Panes),
                Some(style) => {
                    self.status.msg(format!(
                        "mux {style}: invalid style, one of `tabs`, `split`"
                    ));
                }
                None => self.mux.set_mode(self.mux.mode().swap()),
            },
            Some("filter" | "find" | "f") => match parts.next() {
                Some("link") => {
                    self.linked_filters = !self.linked_filters;
                    if self.linked_filters {
                        self.replicate_filters_on_all_viewers();
                    }
                    return true;
                }
                Some("persist") => {
                    let new_persistence = match self.filter_data.is_persistent() {
                        Ok(persistence) => !persistence,
                        Err(err) => {
                            self.status.msg(format!("filter persist: {err}"));
                            return true;
                        }
                    };

                    if let Err(err) = self.filter_data.set_persistent(new_persistence) {
                        self.status.msg(format!("filter persist: {err}"));
                        return true;
                    }

                    self.status
                        .msg(format!("filter persist: persistence = {new_persistence}"));
                }
                Some("copy" | "c") => {
                    let Some(source) = self.mux.active_viewer_mut() else {
                        return true;
                    };
                    let export = source.compositor_mut().export_user_filters();

                    let Some(idx) = parts.next() else {
                        self.status
                            .msg(String::from("filter export: requires instance index"));
                        return true;
                    };

                    let Ok(idx) = idx.parse::<usize>() else {
                        self.status
                            .msg(format!("filter export {idx}: invalid index"));
                        return true;
                    };
                    let idx = idx.saturating_sub(1);
                    if self.mux.active() == idx {
                        self.status.msg(String::from(
                            "filter export: cannot export to active instance",
                        ));
                        return true;
                    }
                    let Some(target) = self.mux.viewers_mut().get_mut(idx) else {
                        self.status
                            .msg(format!("filter export {idx}: invalid index"));
                        return true;
                    };

                    target.import_user_filters(&export);
                }
                Some("save") => {
                    let Some(source) = self.mux.active_viewer_mut() else {
                        return true;
                    };
                    let export = source.compositor_mut().export_user_filters();

                    if let Err(err) = self.filter_data.add_filter(export) {
                        self.status.msg(format!("filter save: {err}"));
                    }

                    self.status.msg("filter save: saved filters".to_string());
                }

                Some("load") => {
                    let filter_sets = match self.filter_data.filters() {
                        Ok(filters) => filters,
                        Err(err) => {
                            self.status.msg(format!("filter save: {err}"));
                            return true;
                        }
                    };

                    match filter_sets.first() {
                        Some(export) => {
                            // Can get rid of this clone if process_filter_actions was part of mux
                            let export = export.clone();
                            self.process_filter_action(|viewer| {
                                viewer.clear_filters();
                                viewer.import_user_filters(&export);
                            });
                        }
                        None => {
                            self.status.msg("filter load: no saved filters".to_string());
                        }
                    }
                }
                Some("regex" | "r") => {
                    let pat = parts.collect::<String>();
                    return self.process_search(&pat, false);
                }
                Some("literal" | "lit" | "l") => {
                    let pat = parts.collect::<String>();
                    return self.process_search(&pat, true);
                }
                Some("clear") => {
                    self.process_filter_action(|viewer| {
                        viewer.clear_filters();
                    });
                }
                Some("union" | "u" | "||" | "|") => {
                    self.process_filter_action(|viewer| {
                        viewer.set_composite_strategy(CompositeStrategy::Union);
                    });
                }
                Some("intersect" | "i" | "&&" | "&") => {
                    self.process_filter_action(|viewer| {
                        viewer.set_composite_strategy(CompositeStrategy::Intersection);
                    });
                }
                Some(cmd) => {
                    self.status.msg(format!("filter {cmd}: invalid subcommand"));
                }
                None => {
                    self.status.msg(
                        String::from("filter: requires subcommand, one of `r[egex]`, `l[it]`, `clear`, `union`, `intersect`")
                    );
                }
            },
            Some("export") => {
                let path = parts.collect::<PathBuf>();
                self.status.msg(format!(
                    "{}: export starting (this may take a while...)",
                    path.display()
                ));
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
                    self.status.msg(format!("{cmd}: Invalid command"))
                }
            }
            None => return true,
        }

        true
    }

    fn ui(&mut self, handler: &mut MouseHandler) -> Option<(u16, u16)> {
        let mut f = self.term.get_frame();
        let [mux_chunk, cmd_chunk] = MultiplexerWidget::split_bottom(f.area(), 1);

        match self.mode {
            InputMode::Prompt(PromptMode::Search { escaped }) => {
                let pattern = self.prompt.buf();

                let pattern_mismatch = self
                    .regex_cache
                    .as_ref()
                    .map(|cache| cache.escaped != escaped || cache.pattern != pattern)
                    .unwrap_or(true);

                if pattern_mismatch {
                    let regex = if !escaped {
                        regex_compile(pattern)
                    } else {
                        regex_compile(&regex::escape(pattern))
                    }
                    .ok();

                    self.regex_cache = Some(RegexCache {
                        pattern: pattern.to_owned(),
                        escaped,
                        regex,
                    })
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
            linked_filters: self.linked_filters,
            regex: self
                .regex_cache
                .as_ref()
                .and_then(|cache| cache.regex.as_ref()),
        }
        .render(mux_chunk, f.buffer_mut(), handler);

        let mut cursor = None;
        PromptWidget {
            mode: self.mode,
            inner: &mut self.prompt,
            cursor: &mut cursor,
        }
        .render(cmd_chunk, f.buffer_mut());

        cursor
    }
}
