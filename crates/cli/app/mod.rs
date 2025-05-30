mod actions;
pub mod control;
mod keybinding;
mod mouse;
mod widgets;

use self::{
    actions::{Action, CommandAction, NormalAction, VisualAction},
    control::{InputMode, PromptMode},
    keybinding::Keybinding,
    mouse::MouseHandler,
    widgets::{MultiplexerWidget, PromptWidget},
};
use crate::{
    components::{
        config::filter::FilterConfigApp,
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

struct RegexCache {
    pattern: String,
    escaped: bool,
    regex: Option<Regex>,
}

pub struct App<'term> {
    term: Terminal<'term>,

    viewer: Viewer,

    keybinds: Keybinding,
    clipboard: Option<Clipboard>,
    action_queue: VecDeque<Action>,

    mouse_capture: bool,
    refresh: bool,
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
            viewer: Viewer::new(),
            keybinds: Keybinding::Hardcoded,
            clipboard: Clipboard::new().ok(),
            action_queue: VecDeque::new(),
            mouse_capture: true,
            refresh: false,
        }
    }

    pub fn viewer_mut(&mut self) -> &mut Viewer {
        &mut self.viewer
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

        if self.viewer.filter_config.is_persistent() {
            if let Some(source) = self.viewer.mux.active_mut() {
                let export = source.compositor_mut().filters().export(None);

                if let Err(err) = self.viewer.filter_config.set_persistent_filter(export) {
                    self.viewer.status.msg(format!("filter save: {err}"));
                }

                self.viewer
                    .status
                    .msg("filter save: saved filters".to_string());
            }
        }

        result
    }

    fn event_loop(&mut self) -> Result<()> {
        let mut mouse_handler = MouseHandler::new();

        loop {
            if self.refresh {
                self.term.clear()?;
                self.refresh = false;
            }
            self.term.draw(|f| {
                let cursor = self.viewer.ui(f, &mut mouse_handler);
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
                        let key = self.keybinds.map_key(self.viewer.mode, &mut event);
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

    fn process_action(&mut self, action: Action) -> Result<bool> {
        match action {
            Action::Exit => return Ok(false),
            Action::SwitchMode(new_mode) => {
                let old_mode = self.viewer.mode;
                self.viewer.mode = new_mode;

                match new_mode {
                    InputMode::Visual => {
                        if let Some(instance) = self.viewer.mux.active_mut() {
                            instance.move_selected_into_view();
                            instance.set_follow_output(false);
                        }
                    }
                    InputMode::Prompt(PromptMode::Search { edit: true, .. }) => {
                        if let InputMode::Prompt(PromptMode::Search { edit: true, .. }) = old_mode {
                            return Ok(true);
                        }
                        match self
                            .viewer
                            .mux
                            .active_mut()
                            .and_then(|instance| instance.compositor_mut().selected_filter())
                            .and_then(|filter| filter.mask().regex())
                        {
                            Some(regex) => {
                                self.viewer.prompt.take();
                                self.viewer.prompt.enter_str(regex.as_str());
                            }
                            _ => {
                                self.viewer.mode = old_mode;
                                return Ok(true);
                            }
                        };
                    }
                    _ => {
                        if !old_mode.is_prompt_search() || !new_mode.is_prompt_search() {
                            self.viewer.prompt.take();
                        }
                    }
                }
            }
            Action::Normal(action) => match action {
                NormalAction::PanVertical {
                    direction,
                    delta,
                    target_view,
                } => {
                    if let Some(instance) = self.viewer.get_target_view(target_view) {
                        instance.move_viewport_vertical(direction, delta)
                    }
                }
                NormalAction::PanHorizontal {
                    direction,
                    delta,
                    target_view,
                } => {
                    if let Some(instance) = self.viewer.get_target_view(target_view) {
                        instance.move_viewport_horizontal(direction, delta)
                    }
                }
                NormalAction::FollowOutput => {
                    if let Some(instance) = self.viewer.mux.active_mut() {
                        instance.set_follow_output(true);
                    }
                }
                NormalAction::SwitchActiveIndex { target_view } => {
                    self.viewer.mux.move_active_index(target_view)
                }
                NormalAction::SwitchActive(direction) => self.viewer.mux.move_active(direction),
            },
            Action::Visual(action) => match action {
                VisualAction::Move {
                    direction,
                    select,
                    delta,
                } => {
                    if let Some(instance) = self.viewer.mux.active_mut() {
                        instance.move_select(direction, select, delta);
                        instance.set_follow_output(false);
                    }
                }
                VisualAction::ToggleSelectedLine => {
                    if let Some(instance) = self.viewer.mux.active_mut() {
                        instance.toggle_select_bookmarks();
                    }
                }
                VisualAction::ToggleLine {
                    target_view,
                    line_number,
                } => {
                    if let Some(instance) = self.viewer.mux.instances_mut().get_mut(target_view) {
                        instance.toggle_bookmark_line_number(line_number)
                    }
                }
            },
            Action::Filter(action) => match action {
                actions::FilterAction::Move {
                    direction,
                    select,
                    delta,
                } => {
                    self.viewer
                        .mux
                        .demux_mut(self.viewer.linked_filters, |instance| {
                            instance
                                .compositor_mut()
                                .move_select(direction, select, delta)
                        });
                }
                actions::FilterAction::ToggleSelectedFilter => {
                    self.viewer
                        .mux
                        .demux_mut(self.viewer.linked_filters, |instance| {
                            let selected_filters =
                                instance.compositor_mut().selected_filter_indices();
                            instance.toggle_filters(selected_filters);
                        });
                }
                actions::FilterAction::RemoveSelectedFilter => {
                    self.viewer
                        .mux
                        .demux_mut(self.viewer.linked_filters, |instance| {
                            let selected_filters =
                                instance.compositor_mut().selected_filter_indices();
                            instance.remove_filters(selected_filters);
                        });
                }
                actions::FilterAction::ToggleFilter {
                    target_view,
                    filter_index,
                } => {
                    if self.viewer.linked_filters {
                        // TODO: handle this
                        return Ok(true);
                    }
                    if let Some(instance) = self.viewer.mux.instances_mut().get_mut(target_view) {
                        instance.toggle_filter(filter_index)
                    }
                }
            },
            Action::Config(action) => match action {
                actions::ConfigAction::Move {
                    direction,
                    select,
                    delta,
                } => self
                    .viewer
                    .filter_config
                    .move_select(direction, select, delta),
                actions::ConfigAction::LoadSelectedFilter => {
                    let Some(export) = self.viewer.filter_config.selected_filter() else {
                        return Ok(true);
                    };

                    self.viewer
                        .mux
                        .demux_mut(self.viewer.linked_filters, |target| {
                            target.import_user_filters(&export);
                        });
                }
                actions::ConfigAction::RemoveSelectedFilter => {
                    let selected_filters = self.viewer.filter_config.selected_filter_indices();
                    if let Err(err) = self.viewer.filter_config.remove_filters(selected_filters) {
                        self.viewer.status.msg(format!("filter save remove: {err}"));
                    }
                }
            },
            Action::Command(action) => match action {
                CommandAction::Move {
                    direction,
                    select,
                    jump,
                } => self.viewer.prompt.move_cursor(
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
                CommandAction::Type { input } => self.viewer.prompt.enter_char(input),
                CommandAction::Paste { input } => self.viewer.prompt.enter_str(&input),
                CommandAction::Backspace => {
                    if !self.viewer.prompt.delete() {
                        self.viewer.mode = InputMode::Normal;
                    }
                }
                CommandAction::Submit => {
                    let result = match self.viewer.mode {
                        InputMode::Prompt(PromptMode::Command) => {
                            self.viewer.mode = InputMode::Normal;
                            let command = self.viewer.prompt.submit();
                            Ok(self.process_command(&command))
                        }
                        InputMode::Prompt(PromptMode::Search { escaped, edit }) => {
                            self.viewer.mode = InputMode::Normal;
                            let command = self.viewer.prompt.take();
                            Ok(self.process_search(&command, escaped, edit))
                        }
                        InputMode::Prompt(PromptMode::Shell { pipe }) => {
                            self.viewer.mode = InputMode::Normal;
                            let command = self.viewer.prompt.take();
                            self.process_shell(&command, true, pipe)
                        }
                        InputMode::Normal
                        | InputMode::Visual
                        | InputMode::Filter
                        | InputMode::Config => unreachable!(),
                    };
                    return result;
                }
                CommandAction::History { direction } => {
                    if self.viewer.mode != InputMode::Prompt(PromptMode::Command) {
                        return Ok(true);
                    }
                    match direction {
                        Direction::Back => self.viewer.prompt.backward(),
                        Direction::Next => self.viewer.prompt.forward(),
                    }
                }
                CommandAction::Complete => (),
            },
            Action::ExportFile(path) => {
                if let Some(instance) = self.viewer.mux.active_mut() {
                    if let Err(err) = OpenOptions::new()
                        .create_new(true)
                        .write(true)
                        .truncate(true)
                        .open(&path)
                        .map_err(Error::from)
                        .and_then(|mut file| instance.write_bytes(&mut file))
                    {
                        self.viewer.status.msg(format!("{}: {err}", path.display()));
                    } else {
                        self.viewer
                            .status
                            .msg(format!("{}: export complete", path.display()));
                    }
                } else {
                    self.viewer.status.msg(String::from("No active instances"));
                }
            }
        };

        Ok(true)
    }

    fn context(&mut self, s: &str) -> Result<Option<Cow<'static, str>>, std::env::VarError> {
        match s {
            "SEL" | "sel" => {
                if let Some(instance) = self.viewer.mux.active_mut() {
                    match instance.export_string() {
                        Ok(text) => Ok(Some(text.into())),
                        Err(err) => {
                            self.viewer
                                .status
                                .msg(format!("selection expansion: {err}"));
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

    fn process_shell(&mut self, command: &str, terminate: bool, pipe: bool) -> Result<bool> {
        let Ok(expanded) = shellexpand::env_with_context(command, |s| self.context(s)) else {
            self.viewer
                .status
                .msg("shell: expansion failed".to_string());
            return Ok(true);
        };

        let mut shl = shlex::Shlex::new(&expanded);
        let Some(cmd) = shl.next() else {
            self.viewer
                .status
                .msg("shell: no command provided".to_string());
            return Ok(true);
        };

        let args = shl.by_ref().collect::<Vec<_>>();

        if shl.had_error {
            self.viewer.status.msg("shell: lexing failed".to_string());
            return Ok(true);
        }

        let mut command = std::process::Command::new(cmd);
        command.args(args);

        self.exit_terminal()?;
        let mut child = match command.spawn() {
            Err(err) => {
                self.term.clear()?;
                self.enter_terminal()?;
                self.viewer.status.msg(format!("shell: {err}"));
                return Ok(true);
            }
            Ok(child) => child,
        };

        if pipe {
            let mut stdin = child.stdin.take().unwrap();
            if let Some(instance) = self.viewer.mux.active_mut() {
                instance.write_bytes(&mut stdin)?;
            }
        }

        if terminate {
            self.viewer.mux.clear();
        }

        let status = match child.wait() {
            Err(err) => {
                self.viewer.status.msg(format!("shell: {err}"));
                return Ok(true);
            }
            Ok(status) => status,
        };

        if terminate {
            std::process::exit(status.code().unwrap_or(0));
        }

        Ok(!terminate)
    }

    fn process_search(&mut self, pat: &str, escaped: bool, edit: bool) -> bool {
        let mut e = None;
        self.viewer
            .mux
            .demux_mut(self.viewer.linked_filters, |instance| {
                let result = if edit {
                    instance.edit_search_filter(pat, escaped)
                } else {
                    instance.add_search_filter(pat, escaped)
                };
                if let Err(err) = result {
                    e.get_or_insert(err);
                };
            });

        if let Some(err) = e {
            self.viewer.status.msg(match err {
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
                    self.viewer
                        .status
                        .msg("mouse capture toggle failed".to_string());
                }
                return true;
            }
            Some("refresh") => {
                self.refresh = true;
            }
            Some("open" | "o") => {
                let path = parts.collect::<PathBuf>();
                if let Err(err) = self.viewer.open_file(path.as_ref()) {
                    self.viewer.status.msg(format!("{}: {err}", path.display()));
                }
            }
            Some("pb" | "pbcopy") => {
                let Some(clipboard) = self.clipboard.as_mut() else {
                    self.viewer
                        .status
                        .msg("pbcopy: clipboard not available".to_string());
                    return true;
                };
                if let Some(instance) = self.viewer.mux.active_mut() {
                    match instance.export_string() {
                        Ok(text) => match clipboard.set_text(text) {
                            Ok(_) => {
                                self.viewer
                                    .status
                                    .msg("pbcopy: copied to clipboard".to_string());
                            }
                            Err(err) => {
                                self.viewer.status.msg(format!("pbcopy: {err}"));
                            }
                        },
                        Err(err) => {
                            self.viewer.status.msg(format!("pbcopy: {err}"));
                        }
                    }
                }
            }
            Some("close" | "c") => {
                if self.viewer.mux.active_mut().is_some() {
                    self.viewer.mux.close_active()
                } else {
                    self.viewer.status.msg(String::from("No active instances"));
                }
            }
            Some("gutter" | "g") => {
                self.viewer.gutter = !self.viewer.gutter;
            }
            Some("mux" | "m") => match parts.next() {
                Some("tabs" | "t" | "none") => self.viewer.mux.set_mode(MultiplexerMode::Tabs),
                Some("split" | "s" | "win") => self.viewer.mux.set_mode(MultiplexerMode::Panes),
                Some(style) => {
                    self.viewer.status.msg(format!(
                        "mux {style}: invalid style, one of `tabs`, `split`"
                    ));
                }
                None => self.viewer.mux.set_mode(self.viewer.mux.mode().swap()),
            },
            Some("filter" | "find" | "f") => match parts.next() {
                Some("link") => {
                    self.viewer.linked_filters = !self.viewer.linked_filters;
                    if self.viewer.linked_filters {
                        self.viewer.replicate_filters_on_all_instances();
                    }
                    return true;
                }
                Some("persist") => {
                    let new_persistence = !self.viewer.filter_config.is_persistent();

                    if let Err(err) = self.viewer.filter_config.set_persistent(new_persistence) {
                        self.viewer.status.msg(format!("filter persist: {err}"));
                        return true;
                    }

                    self.viewer
                        .status
                        .msg(format!("filter persist: persistence = {new_persistence}"));
                }
                Some("copy" | "c") => {
                    let Some(source) = self.viewer.mux.active_mut() else {
                        return true;
                    };
                    let export = source.compositor_mut().filters().export(None);

                    let Some(idx) = parts.next() else {
                        self.viewer
                            .status
                            .msg(String::from("filter export: requires instance index"));
                        return true;
                    };

                    let Ok(idx) = idx.parse::<usize>() else {
                        self.viewer
                            .status
                            .msg(format!("filter export {idx}: invalid index"));
                        return true;
                    };
                    let idx = idx.saturating_sub(1);
                    if self.viewer.mux.active_index() == idx {
                        self.viewer.status.msg(String::from(
                            "filter export: cannot export to active instance",
                        ));
                        return true;
                    }
                    let Some(target) = self.viewer.mux.instances_mut().get_mut(idx) else {
                        self.viewer
                            .status
                            .msg(format!("filter export {idx}: invalid index"));
                        return true;
                    };

                    target.import_user_filters(&export);
                }
                Some("save") => {
                    let Some(source) = self.viewer.mux.active_mut() else {
                        return true;
                    };
                    let name: String = parts.collect::<Vec<&str>>().join(" ");
                    let export = source.compositor_mut().filters().export(Some(name));

                    if let Err(err) = self.viewer.filter_config.add_filter(export) {
                        self.viewer.status.msg(format!("filter save: {err}"));
                    }

                    self.viewer
                        .status
                        .msg("filter save: saved filters".to_string());
                }
                Some("load") => {
                    self.viewer.mode = InputMode::Config;
                }
                Some("clear") => {
                    self.viewer
                        .mux
                        .demux_mut(self.viewer.linked_filters, |instance| {
                            instance.clear_filters();
                        });
                }
                Some("union" | "u" | "||" | "|") => {
                    self.viewer
                        .mux
                        .demux_mut(self.viewer.linked_filters, |instance| {
                            instance.set_composite_strategy(CompositeStrategy::Union);
                        });
                }
                Some("intersect" | "i" | "&&" | "&") => {
                    self.viewer
                        .mux
                        .demux_mut(self.viewer.linked_filters, |instance| {
                            instance.set_composite_strategy(CompositeStrategy::Intersection);
                        });
                }
                Some(cmd) => {
                    self.viewer
                        .status
                        .msg(format!("filter {cmd}: invalid subcommand"));
                }
                None => {
                    self.viewer.status.msg(
                        String::from("filter: requires subcommand, one of `r[egex]`, `l[it]`, `clear`, `union`, `intersect`")
                    );
                }
            },
            Some("export") => {
                let path = parts.collect::<PathBuf>();
                self.viewer.status.msg(format!(
                    "{}: export starting (this may take a while...)",
                    path.display()
                ));
                self.action_queue.push_back(Action::ExportFile(path));
            }
            Some(cmd) => {
                if let Ok(line_number) = cmd.parse::<usize>() {
                    if let Some(instance) = self.viewer.mux.active_mut() {
                        if let Some(idx) = instance.nearest_index(line_number) {
                            instance.viewport_mut().jump_vertically_to(idx);
                        }
                    }
                } else {
                    self.viewer.status.msg(format!("{cmd}: Invalid command"))
                }
            }
            None => return true,
        }

        true
    }
}

pub struct Viewer {
    mode: InputMode,
    mux: MultiplexerApp,
    status: StatusApp,
    prompt: PromptApp,
    regex_cache: Option<RegexCache>,
    filter_config: FilterConfigApp,
    gutter: bool,
    linked_filters: bool,
}

impl Viewer {
    pub fn new() -> Self {
        Self {
            mode: InputMode::Normal,
            prompt: PromptApp::new(),
            mux: MultiplexerApp::new(),
            status: StatusApp::new(),
            regex_cache: None,
            filter_config: FilterConfigApp::new(),
            gutter: true,
            linked_filters: false,
        }
    }

    fn push_instance(&mut self, name: String, file: SegBuffer) {
        self.mux.push(Instance::new(name, file));
    }

    pub fn open_file(&mut self, path: &Path) -> Result<()> {
        let load_filters = self.mux.is_empty() && self.filter_config.is_persistent();

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
            let filter_set = match self.filter_config.get_persistent_filter() {
                Ok(filters) => filters,
                Err(err) => {
                    self.status.msg(format!("filter persist/load: {err}"));
                    return Ok(());
                }
            };
            let instance = self.mux.active_mut().unwrap();
            match filter_set {
                Some(export) => instance.import_user_filters(export),
                None => {}
            }
        }
        if self.linked_filters {
            if let Some(source) = self.mux.active_mut() {
                let export = source.compositor_mut().filters().export(None);
                let cursor = *source.compositor_mut().cursor();

                let instance = self.mux.instances_mut().last_mut().unwrap();
                instance.import_user_filters(&export);
                instance.compositor_mut().set_cursor(cursor)
            }
        }

        Ok(())
    }

    pub fn open_stream(&mut self, name: String, stream: BoxedStream) -> Result<()> {
        self.push_instance(name, SegBuffer::read_stream(stream, false)?);
        Ok(())
    }

    fn get_target_view(&mut self, target_view: Option<usize>) -> Option<&mut Instance> {
        if let Some(index) = target_view {
            self.mux.instances_mut().get_mut(index)
        } else {
            self.mux.active_mut()
        }
    }

    fn replicate_filters_on_all_instances(&mut self) {
        if let Some(source) = self.mux.active_mut() {
            let export = source.compositor_mut().filters().export(None);
            let cursor = *source.compositor_mut().cursor();
            let active = self.mux.active_index();
            self.mux
                .instances_mut()
                .iter_mut()
                .enumerate()
                .filter(|(i, _)| *i != active)
                .for_each(|(_, instance)| {
                    instance.import_user_filters(&export);
                    instance.compositor_mut().set_cursor(cursor)
                });
        }
    }

    fn ui(&mut self, f: &mut ratatui::Frame, handler: &mut MouseHandler) -> Option<(u16, u16)> {
        let [mux_chunk, cmd_chunk] = MultiplexerWidget::split_bottom(f.area(), 1);

        match self.mode {
            InputMode::Prompt(PromptMode::Search { escaped, .. }) => {
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
            InputMode::Prompt(_)
            | InputMode::Normal
            | InputMode::Visual
            | InputMode::Filter
            | InputMode::Config => {
                self.regex_cache = None;
            }
        }

        MultiplexerWidget {
            mux: &mut self.mux,
            status: &mut self.status,
            mode: self.mode,
            config: &mut self.filter_config,
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
