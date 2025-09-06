mod actions;
pub mod control;
mod keybinding;
mod mouse;
mod terminal;
mod widgets;

use self::{
    actions::{Action, CommandAction, NormalAction, VisualAction},
    control::{InputMode, PromptMode},
    keybinding::Keybinding,
    mouse::MouseHandler,
};
use crate::{
    app::{
        terminal::{Terminal, TerminalState},
        widgets::{MultiplexerWidget, PromptWidget},
    },
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
use actions::{ConfigAction, FilterAction};
use anyhow::Result;
use bvr_core::{SegBuffer, err::Error, index::BoxedStream, matches::CompositeStrategy};
use crossterm::{clipboard::CopyToClipboard, event};
use regex::bytes::Regex;
use std::{
    borrow::Cow,
    collections::VecDeque,
    fs::OpenOptions,
    num::NonZeroUsize,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub struct State {
    viewer: Viewer,
    keybinds: Keybinding,
}

impl State {
    pub fn new() -> Self {
        Self {
            viewer: Viewer::new(),
            keybinds: Keybinding::Hardcoded,
        }
    }

    pub fn viewer_mut(&mut self) -> &mut Viewer {
        &mut self.viewer
    }
}

impl App {
    pub fn new(app: State, term: Terminal) -> Self {
        Self {
            app,
            term: TerminalState::new(term),
            action_queue: VecDeque::new(),
            refresh: false,
        }
    }

    pub fn run(&mut self) -> Result<()> {
        self.term.enter_terminal()?;

        self.event_loop()?;

        if self.app.viewer.filter_config.is_persistent() {
            if let Some(source) = self.app.viewer.mux.active_mut() {
                let export = source.compositor_mut().filters().export(None);

                if let Err(err) = self.app.viewer.filter_config.set_persistent_filter(export) {
                    self.app.viewer.status.msg(format!("filter save: {err}"));
                }

                self.app
                    .viewer
                    .status
                    .msg("filter save: saved filters".to_string());
            }
        }

        Ok(())
    }

    fn event_loop(&mut self) -> Result<()> {
        let mut mouse_handler = MouseHandler::new();

        let mut last_drawn: Option<Instant> = None;
        loop {
            if self.refresh {
                self.term.clear()?;
                self.refresh = false;
            }

            let mut render = |f: &mut ratatui::Frame| self.app.viewer.ui(f, &mut mouse_handler);

            const MIN_REFRESH_DURATION: Duration = Duration::from_millis(16);
            const MIN_POLL_DURATION: Duration = Duration::from_millis(32);

            let now = Instant::now();

            if last_drawn
                .map(|last_drawn| now.duration_since(last_drawn) > MIN_REFRESH_DURATION)
                .unwrap_or(true)
            {
                self.term.draw(|f| {
                    let cursor = render(f);
                    if let Some(cursor) = cursor {
                        f.set_cursor_position(cursor);
                    }
                })?;
                last_drawn = Some(now);
            } else if self.term.mouse_capture {
                // We render to capture mouse actions
                render(&mut self.term.get_frame());
                // But we avoid drawing so terminal won't look weird
                self.term.current_buffer_mut().reset();
            }

            let action = match self
                .action_queue
                .pop_front()
                .or_else(|| mouse_handler.extract())
            {
                Some(action) => action,
                None if event::poll(MIN_POLL_DURATION)? => {
                    let mut event = event::read()?;
                    let key = self.app.keybinds.map_key(self.app.viewer.mode, &mut event);
                    mouse_handler.publish_event(event);
                    let Some(action) = key else { continue };
                    action
                }
                None => continue,
            };

            if !self.process_action(action)? {
                break Ok(());
            }
        }
    }

    fn process_action(&mut self, action: Action) -> Result<bool> {
        match action {
            Action::Exit => return Ok(false),
            Action::SwitchMode(new_mode) => {
                let old_mode = self.app.viewer.mode;
                self.app.viewer.mode = new_mode;

                match new_mode {
                    InputMode::Visual => {
                        if let Some(instance) = self.app.viewer.mux.active_mut() {
                            instance.move_selected_into_view();
                            instance.set_follow_output(false);
                        }
                    }
                    InputMode::Prompt(PromptMode::Search { edit: true, .. }) => {
                        if let InputMode::Prompt(PromptMode::Search { edit: true, .. }) = old_mode {
                            return Ok(true);
                        }
                        match self
                            .app
                            .viewer
                            .mux
                            .active_mut()
                            .and_then(|instance| instance.compositor_mut().selected_filter())
                            .and_then(|filter| filter.mask().regex())
                        {
                            Some(regex) => {
                                self.app.viewer.prompt.take();
                                self.app.viewer.prompt.enter_str(regex.as_str());
                            }
                            _ => {
                                self.app.viewer.mode = old_mode;
                                return Ok(true);
                            }
                        };
                    }
                    _ => {
                        if !old_mode.is_prompt_search() || !new_mode.is_prompt_search() {
                            self.app.viewer.prompt.take();
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
                    if let Some(instance) = self.app.viewer.get_target_view(target_view) {
                        instance.move_viewport_vertical(direction, delta)
                    }
                }
                NormalAction::PanHorizontal {
                    direction,
                    delta,
                    target_view,
                } => {
                    if let Some(instance) = self.app.viewer.get_target_view(target_view) {
                        instance.move_viewport_horizontal(direction, delta)
                    }
                }
                NormalAction::FollowOutput => {
                    if let Some(instance) = self.app.viewer.mux.active_mut() {
                        instance.set_follow_output(true);
                    }
                }
                NormalAction::SwitchActiveIndex { target_view } => {
                    self.app.viewer.mux.move_active_index(target_view)
                }
                NormalAction::SwitchActive(direction) => self.app.viewer.mux.move_active(direction),
            },
            Action::Visual(action) => match action {
                VisualAction::Move {
                    direction,
                    select,
                    delta,
                } => {
                    if let Some(instance) = self.app.viewer.mux.active_mut() {
                        instance.move_select(direction, select, delta);
                        instance.set_follow_output(false);
                    }
                }
                VisualAction::ToggleSelectedLine => {
                    if let Some(instance) = self.app.viewer.mux.active_mut() {
                        instance.toggle_select_bookmarks();
                    }
                }
                VisualAction::ToggleLine {
                    target_view,
                    line_number,
                } => {
                    if let Some(instance) = self.app.viewer.mux.instances_mut().get_mut(target_view)
                    {
                        instance.toggle_bookmark_line_number(line_number)
                    }
                }
            },
            Action::Filter(action) => match action {
                FilterAction::Move {
                    direction,
                    select,
                    delta,
                } => {
                    self.app
                        .viewer
                        .mux
                        .demux_mut(self.app.viewer.linked_filters, |instance| {
                            instance
                                .compositor_mut()
                                .move_select(direction, select, delta)
                        });
                }
                FilterAction::ToggleSelectedFilter => {
                    self.app
                        .viewer
                        .mux
                        .demux_mut(self.app.viewer.linked_filters, |instance| {
                            instance.toggle_selected_filters();
                        });
                }
                FilterAction::RemoveSelectedFilter => {
                    self.app
                        .viewer
                        .mux
                        .demux_mut(self.app.viewer.linked_filters, |instance| {
                            instance.remove_selected_filters();
                        });
                }
                FilterAction::Displace { direction, delta } => {
                    self.app
                        .viewer
                        .mux
                        .demux_mut(self.app.viewer.linked_filters, |instance| {
                            instance.displace_selected_filters(direction, delta);
                        });
                }
                FilterAction::ToggleSpecificFilter {
                    target_view,
                    filter_index,
                } => {
                    if self.app.viewer.linked_filters {
                        // TODO: handle this
                        return Ok(true);
                    }
                    if let Some(instance) = self.app.viewer.mux.instances_mut().get_mut(target_view)
                    {
                        instance.toggle_filter(filter_index)
                    }
                }
            },
            Action::Config(action) => match action {
                ConfigAction::Move {
                    direction,
                    select,
                    delta,
                } => self
                    .app
                    .viewer
                    .filter_config
                    .move_select(direction, select, delta),
                ConfigAction::LoadSelectedFilter => {
                    let Some(export) = self.app.viewer.filter_config.selected_filter() else {
                        return Ok(true);
                    };

                    self.app
                        .viewer
                        .mux
                        .demux_mut(self.app.viewer.linked_filters, |target| {
                            target.import_user_filters(&export);
                        });
                }
                ConfigAction::RemoveSelectedFilter => {
                    let selected_filters = self.app.viewer.filter_config.selected_filter_indices();
                    if let Err(err) = self
                        .app
                        .viewer
                        .filter_config
                        .remove_filters(selected_filters)
                    {
                        self.app
                            .viewer
                            .status
                            .msg(format!("filter save remove: {err}"));
                    }
                }
            },
            Action::Command(action) => match action {
                CommandAction::Move {
                    direction,
                    select,
                    jump,
                } => self.app.viewer.prompt.move_cursor(
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
                CommandAction::Type { input } => self.app.viewer.prompt.enter_char(input),
                CommandAction::Paste { input } => self.app.viewer.prompt.enter_str(&input),
                CommandAction::Backspace => {
                    if !self.app.viewer.prompt.delete() {
                        self.app.viewer.mode = InputMode::Normal;
                    }
                }
                CommandAction::Submit => {
                    let result = match self.app.viewer.mode {
                        InputMode::Prompt(PromptMode::Command) => {
                            self.app.viewer.mode = InputMode::Normal;
                            let command = self.app.viewer.prompt.submit();
                            Ok(self.process_command(&command))
                        }
                        InputMode::Prompt(PromptMode::Search { escaped, edit }) => {
                            self.app.viewer.mode = InputMode::Normal;
                            let command = self.app.viewer.prompt.take();
                            Ok(self.process_search(&command, escaped, edit))
                        }
                        InputMode::Prompt(PromptMode::Shell { pipe }) => {
                            self.app.viewer.mode = InputMode::Normal;
                            let command = self.app.viewer.prompt.take();
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
                    if self.app.viewer.mode != InputMode::Prompt(PromptMode::Command) {
                        return Ok(true);
                    }
                    match direction {
                        Direction::Back => self.app.viewer.prompt.backward(),
                        Direction::Next => self.app.viewer.prompt.forward(),
                    }
                }
                CommandAction::Complete => (),
            },
            Action::ExportFile(path) => {
                if let Some(instance) = self.app.viewer.mux.active_mut() {
                    if let Err(err) = OpenOptions::new()
                        .create_new(true)
                        .write(true)
                        .truncate(true)
                        .open(&path)
                        .map_err(Error::from)
                        .and_then(|mut file| instance.write_bytes(&mut file))
                    {
                        self.app
                            .viewer
                            .status
                            .msg(format!("{}: {err}", path.display()));
                    } else {
                        self.app
                            .viewer
                            .status
                            .msg(format!("{}: export complete", path.display()));
                    }
                } else {
                    self.app
                        .viewer
                        .status
                        .msg(String::from("No active instances"));
                }
            }
        };

        Ok(true)
    }

    fn context(&mut self, s: &str) -> Result<Option<Cow<'static, str>>, std::env::VarError> {
        match s {
            "SEL" | "sel" => {
                if let Some(instance) = self.app.viewer.mux.active_mut() {
                    match instance.export_string() {
                        Ok(text) => Ok(Some(text.into())),
                        Err(err) => {
                            self.app
                                .viewer
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
            self.app
                .viewer
                .status
                .msg("shell: expansion failed".to_string());
            return Ok(true);
        };

        let mut shl = shlex::Shlex::new(&expanded);
        let Some(cmd) = shl.next() else {
            self.app
                .viewer
                .status
                .msg("shell: no command provided".to_string());
            return Ok(true);
        };

        let args = shl.by_ref().collect::<Vec<_>>();

        if shl.had_error {
            self.app
                .viewer
                .status
                .msg("shell: lexing failed".to_string());
            return Ok(true);
        }

        let mut command = std::process::Command::new(cmd);
        command.args(args);

        self.term.exit_terminal()?;
        let mut child = match command.spawn() {
            Err(err) => {
                self.term.clear()?;
                self.term.enter_terminal()?;
                self.app.viewer.status.msg(format!("shell: {err}"));
                return Ok(true);
            }
            Ok(child) => child,
        };

        if pipe {
            let mut stdin = child.stdin.take().unwrap();
            if let Some(instance) = self.app.viewer.mux.active_mut() {
                instance.write_bytes(&mut stdin)?;
            }
        }

        if terminate {
            self.app.viewer.mux.clear();
        }

        let status = match child.wait() {
            Err(err) => {
                self.app.viewer.status.msg(format!("shell: {err}"));
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
        if pat.is_empty() {
            return true;
        }

        let mut e = None;
        self.app
            .viewer
            .mux
            .demux_mut(self.app.viewer.linked_filters, |instance| {
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
            self.app.viewer.status.msg(match err {
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
                if let Err(_) = self.term.toggle_mouse_capture() {
                    self.app
                        .viewer
                        .status
                        .msg("mouse capture toggle failed".to_string());
                }
                return true;
            }
            Some("readlink" | "realpath" | "rl" | "rp") => {
                if let Some(instance) = self.app.viewer.mux.active_mut() {
                    if let Some(link) = instance.link() {
                        let link = link.display();
                        self.app.viewer.status.msg(format!("readlink: {}", link));
                        crossterm::execute!(
                            self.term.backend_mut(),
                            CopyToClipboard::to_clipboard_from(link.to_string())
                        )
                        .ok();
                    } else {
                        self.app.viewer.status.msg("readlink: no link".to_string());
                    }
                } else {
                    self.app
                        .viewer
                        .status
                        .msg(String::from("No active instances"));
                }
            }
            Some("refresh") => {
                self.refresh = true;
            }
            Some("open" | "o") => {
                let path = parts.collect::<PathBuf>();
                if let Err(err) = self.app.viewer.open_file(path.as_ref()) {
                    self.app
                        .viewer
                        .status
                        .msg(format!("{}: {err}", path.display()));
                }
            }
            Some("pb" | "pbcopy") => {
                if let Some(instance) = self.app.viewer.mux.active_mut() {
                    match instance.export_string() {
                        Ok(text) => {
                            match crossterm::execute!(
                                self.term.backend_mut(),
                                CopyToClipboard::to_clipboard_from(text)
                            ) {
                                Ok(_) => {
                                    self.app
                                        .viewer
                                        .status
                                        .msg("pbcopy: copied to clipboard".to_string());
                                }
                                Err(err) => {
                                    self.app.viewer.status.msg(format!("pbcopy: {err}"));
                                }
                            };
                        }
                        Err(err) => {
                            self.app.viewer.status.msg(format!("pbcopy: {err}"));
                        }
                    }
                }
            }
            Some("close" | "c") => {
                if self.app.viewer.mux.active_mut().is_some() {
                    self.app.viewer.mux.close_active()
                } else {
                    self.app
                        .viewer
                        .status
                        .msg(String::from("No active instances"));
                }
            }
            Some("gutter" | "g") => {
                self.app.viewer.toggle_gutter();
            }
            Some("mux" | "m") => match parts.next() {
                Some("tabs" | "t" | "none") => self.app.viewer.mux.set_mode(MultiplexerMode::Tabs),
                Some("split" | "s" | "win") => self.app.viewer.mux.set_mode(MultiplexerMode::Panes),
                Some(style) => {
                    self.app.viewer.status.msg(format!(
                        "mux {style}: invalid style, one of `tabs`, `split`"
                    ));
                }
                None => self
                    .app
                    .viewer
                    .mux
                    .set_mode(self.app.viewer.mux.mode().swap()),
            },
            Some("filter" | "find" | "f") => match parts.next() {
                Some("link") => {
                    self.app.viewer.linked_filters = !self.app.viewer.linked_filters;
                    if self.app.viewer.linked_filters {
                        self.app.viewer.replicate_filters_on_all_instances();
                    }
                    return true;
                }
                Some("persist") => {
                    let new_persistence = !self.app.viewer.filter_config.is_persistent();

                    if let Err(err) = self
                        .app
                        .viewer
                        .filter_config
                        .set_persistent(new_persistence)
                    {
                        self.app.viewer.status.msg(format!("filter persist: {err}"));
                        return true;
                    }

                    self.app
                        .viewer
                        .status
                        .msg(format!("filter persist: persistence = {new_persistence}"));
                }
                Some("copy" | "c") => {
                    let Some(source) = self.app.viewer.mux.active_mut() else {
                        return true;
                    };
                    let export = source.compositor_mut().filters().export(None);

                    let Some(idx) = parts.next() else {
                        self.app
                            .viewer
                            .status
                            .msg(String::from("filter export: requires instance index"));
                        return true;
                    };

                    let Ok(idx) = idx.parse::<usize>() else {
                        self.app
                            .viewer
                            .status
                            .msg(format!("filter export {idx}: invalid index"));
                        return true;
                    };
                    let idx = idx.saturating_sub(1);
                    if self.app.viewer.mux.active_index() == idx {
                        self.app.viewer.status.msg(String::from(
                            "filter export: cannot export to active instance",
                        ));
                        return true;
                    }
                    let Some(target) = self.app.viewer.mux.instances_mut().get_mut(idx) else {
                        self.app
                            .viewer
                            .status
                            .msg(format!("filter export {idx}: invalid index"));
                        return true;
                    };

                    target.import_user_filters(&export);
                }
                Some("save") => {
                    let Some(source) = self.app.viewer.mux.active_mut() else {
                        return true;
                    };
                    let name: String = parts.collect::<Vec<&str>>().join(" ");
                    let export = source.compositor_mut().filters().export(Some(name));

                    if let Err(err) = self.app.viewer.filter_config.add_filter(export) {
                        self.app.viewer.status.msg(format!("filter save: {err}"));
                    }

                    self.app
                        .viewer
                        .status
                        .msg("filter save: saved filters".to_string());
                }
                Some("load") => {
                    self.app.viewer.mode = InputMode::Config;
                }
                Some("clear") => {
                    self.app
                        .viewer
                        .mux
                        .demux_mut(self.app.viewer.linked_filters, |instance| {
                            instance.clear_filters();
                        });
                }
                Some("union" | "u" | "||" | "|") => {
                    self.app
                        .viewer
                        .mux
                        .demux_mut(self.app.viewer.linked_filters, |instance| {
                            instance.set_composite_strategy(CompositeStrategy::Union);
                        });
                }
                Some("intersect" | "i" | "&&" | "&") => {
                    self.app
                        .viewer
                        .mux
                        .demux_mut(self.app.viewer.linked_filters, |instance| {
                            instance.set_composite_strategy(CompositeStrategy::Intersection);
                        });
                }
                Some(cmd) => {
                    self.app
                        .viewer
                        .status
                        .msg(format!("filter {cmd}: invalid subcommand"));
                }
                None => {
                    self.app.viewer.status.msg(
                        String::from("filter: requires subcommand, one of `r[egex]`, `l[it]`, `clear`, `union`, `intersect`")
                    );
                }
            },
            Some("export") => {
                let path = parts.collect::<PathBuf>();
                self.app.viewer.status.msg(format!(
                    "{}: export starting (this may take a while...)",
                    path.display()
                ));
                self.action_queue.push_back(Action::ExportFile(path));
            }
            Some(cmd) => {
                if let Ok(line_number) = cmd.parse::<usize>() {
                    if let Some(instance) = self.app.viewer.mux.active_mut() {
                        if let Some(idx) = instance.nearest_index(line_number) {
                            instance.viewport_mut().jump_vertically_to(idx);
                        }
                    }
                } else {
                    self.app
                        .viewer
                        .status
                        .msg(format!("{cmd}: Invalid command"))
                }
            }
            None => return true,
        }

        true
    }
}

struct RegexCache {
    pattern: String,
    escaped: bool,
    regex: Option<Regex>,
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

    fn push_instance(&mut self, name: String, link: Option<PathBuf>, file: SegBuffer) {
        self.mux.push(Instance::new(name, link, file));
    }

    pub fn open_file(&mut self, path: &Path) -> Result<()> {
        let load_filters = self.mux.is_empty() && self.filter_config.is_persistent();

        let file = std::fs::File::open(path)?;

        if !file.metadata()?.is_file() {
            return Err(anyhow::anyhow!("Not a file"));
        }

        let link = std::fs::canonicalize(path).ok();

        let name = path
            .file_name()
            .map(|str| str.to_string_lossy().into_owned())
            .unwrap_or_else(|| String::from("Unnamed File"));
        self.push_instance(
            name,
            link,
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
        self.push_instance(name, None, SegBuffer::read_stream(stream, false)?);
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

    pub fn toggle_gutter(&mut self) {
        self.gutter = !self.gutter;
    }
}

pub struct App {
    app: State,
    term: terminal::TerminalState,
    refresh: bool,
    action_queue: VecDeque<Action>,
}
