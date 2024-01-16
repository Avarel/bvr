use super::{
    actions::{Action, FilterAction, NormalAction},
    mouse::MouseHandler,
    InputMode, PromptMode, ViewDelta,
};
use crate::components::{
    cursor::{Cursor, SelectionOrigin},
    filters::{Filter, FilterData, FilterType},
    instance::{Instance, LineData, LineType},
    mux::{MultiplexerApp, MultiplexerMode},
    prompt::PromptApp,
    status::StatusApp,
};
use crate::{app::actions::VisualAction, colors, direction::Direction};
use crossterm::event::MouseEventKind;
use ratatui::{prelude::*, widgets::*};
use regex::bytes::Regex;

pub struct StatusWidget<'a> {
    input_mode: InputMode,
    viewer: Option<&'a Instance>,
    message: Option<&'a str>,
}

impl<'a> Widget for StatusWidget<'a> {
    fn render(self, mut area: Rect, buf: &mut Buffer) {
        const STATUS_BAR_STYLE: Style = Style::new()
            .fg(colors::STATUS_BAR_TEXT)
            .bg(colors::STATUS_BAR);

        let (accent_color, mode_name) = match self.input_mode {
            InputMode::Prompt(PromptMode::Command) => (colors::COMMAND_ACCENT, " COMMAND "),
            InputMode::Prompt(PromptMode::Shell) => (colors::SHELL_ACCENT, " SHELL "),
            InputMode::Prompt(PromptMode::NewFilter) => (colors::FILTER_ACCENT, " FILTER (RGX) "),
            InputMode::Prompt(PromptMode::NewLit) => (colors::FILTER_ACCENT, " FILTER (LIT) "),
            InputMode::Normal => (colors::VIEWER_ACCENT, " NORMAL "),
            InputMode::Visual => (colors::SELECT_ACCENT, " VISUAL "),
            InputMode::Filter => (colors::FILTER_ACCENT, " FILTER "),
        };

        let mut v = Vec::new();

        v.push(Span::from(mode_name).fg(colors::WHITE).bg(accent_color));
        v.push(Span::raw(" "));

        if let Some(viewer) = &self.viewer {
            v.push(Span::raw(viewer.name()).fg(colors::STATUS_BAR_TEXT));
        } else {
            v.push(Span::raw("Empty").fg(colors::STATUS_BAR_TEXT));
        }
        v.push(Span::raw(" │ ").fg(colors::STATUS_BAR_TEXT));

        if let Some(message) = self.message {
            v.push(Span::raw(message));
        } else if let Some(viewer) = &self.viewer {
            let ln_cnt = viewer.file().line_count();
            let ln_vis = viewer.visible_line_count();
            v.push(Span::raw(format!("{} lines", ln_cnt)).fg(accent_color));
            if ln_vis < ln_cnt {
                v.push(Span::raw(format!(" ({} visible)", ln_vis)).fg(colors::STATUS_BAR_TEXT));
            }
            v.push(Span::raw(" │ ").fg(accent_color));
            v.push(Span::raw(viewer.name()).fg(accent_color));
        } else {
            v.push(Span::raw(":open [file name]").fg(accent_color));
            v.push(Span::raw(" to view a file").fg(colors::STATUS_BAR_TEXT));
        }

        Paragraph::new(Line::from(v))
            .style(STATUS_BAR_STYLE)
            .render(area, buf);

        if let Some(viewer) = self.viewer {
            if viewer.is_following_output() {
                Paragraph::new(Span::raw("Follow  ").fg(colors::STATUS_BAR_TEXT))
            } else {
                let bottom = viewer.viewport().bottom();
                let ln_vis = viewer.visible_line_count();
                let percentage = if ln_vis == 0 {
                    1.0
                } else {
                    bottom as f64 / ln_vis as f64
                }
                .clamp(0.0, 1.0);

                let row = viewer.viewport().top();
                let col = viewer.viewport().left();

                Paragraph::new(Line::from(vec![
                    Span::raw(format!("{}:{}", row + 1, col + 1)).fg(colors::STATUS_BAR_TEXT),
                    Span::raw(format!("  {:.0}%  ", percentage * 100.0))
                        .fg(colors::STATUS_BAR_TEXT),
                ]))
            }
            .alignment(Alignment::Right)
            .render(area, buf)
        }
    }
}

pub struct PromptWidget<'a> {
    pub inner: &'a mut PromptApp,
    pub mode: InputMode,
    pub cursor: &'a mut Option<(u16, u16)>,
}

impl Widget for PromptWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let InputMode::Prompt(mode) = self.mode else {
            const WIDGET_BLOCK: Block = Block::new().style(Style::new().bg(colors::BG));
            WIDGET_BLOCK.render(area, buf);
            return;
        };

        let indicator = match mode {
            PromptMode::Command => Span::raw(":").fg(colors::COMMAND_ACCENT),
            PromptMode::NewFilter => Span::raw("/").fg(colors::FILTER_ACCENT),
            PromptMode::NewLit => Span::raw("?").fg(colors::FILTER_ACCENT),
            PromptMode::Shell => Span::raw("!").fg(colors::SHELL_ACCENT),
        };

        let cursor = self.inner.cursor();
        let left = self.inner.viewport().left();
        let cmd_buf = self.inner.view_and_update(usize::from(area.width));

        let input = Paragraph::new(Line::from(match cursor {
            Cursor::Singleton(_) => {
                vec![indicator, Span::raw(cmd_buf)]
            }
            Cursor::Selection(start, end, _) => vec![
                indicator,
                Span::raw(&cmd_buf[..start.saturating_sub(left)]),
                Span::raw(&cmd_buf[start.saturating_sub(left)..end.saturating_sub(left)])
                    .bg(colors::COMMAND_BAR_SELECT),
                Span::raw(&cmd_buf[end.saturating_sub(left)..]),
            ],
        }))
        .bg(colors::BG);

        let i = match cursor {
            Cursor::Singleton(i)
            | Cursor::Selection(_, i, SelectionOrigin::Right)
            | Cursor::Selection(i, _, SelectionOrigin::Left) => {
                cmd_buf[..i.saturating_sub(left)].chars().count()
            }
        };
        *self.cursor = Some((area.x + i as u16 + 1, area.y));

        input.render(area, buf);
    }
}

pub struct FilterViewerWidget<'a> {
    view_index: usize,
    viewer: &'a mut Instance,
}

impl FilterViewerWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer, handle: &mut MouseHandler) {
        const WIDGET_BLOCK: Block = Block::new().style(Style::new().bg(colors::STATUS_BAR));
        WIDGET_BLOCK.render(area, buf);

        let mut y = area.y;
        for filter in self
            .viewer
            .compositor_mut()
            .update_and_filter_view(area.height as usize)
        {
            FilterLineWidget {
                view_index: self.view_index,
                inner: &filter,
            }
            .render(Rect::new(area.x, y, area.width, 1), buf, handle);
            y += 1;
        }
    }
}

pub struct ViewerWidget<'a> {
    view_index: usize,
    viewer: &'a mut Instance,
    show_selection: bool,
    gutter: bool,
    regex: Option<&'a Regex>,
}

impl ViewerWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer, handle: &mut MouseHandler) {
        let left = self.viewer.viewport().left();
        let (view, last_line) = self
            .viewer
            .update_and_view(area.height as usize, area.width as usize);

        let gutter_size = self.gutter.then(|| {
            last_line
                .map(|ln| ((ln + 1).ilog10() + 1) as u16)
                .unwrap_or_default()
                .max(4)
        });

        let mut itoa_buf = itoa::Buffer::new();
        let mut y = area.y;
        for line in view.into_iter() {
            ViewerLineWidget {
                view_index: self.view_index,
                start: left,
                line: Some(line),
                show_selection: self.show_selection,
                itoa_buf: &mut itoa_buf,
                gutter_size,
                regex: self.regex,
            }
            .render(Rect::new(area.x, y, area.width, 1), buf, handle);
            y += 1;
        }

        while y < area.bottom() {
            ViewerLineWidget {
                view_index: self.view_index,
                start: left,
                line: None,
                show_selection: self.show_selection,
                itoa_buf: &mut itoa_buf,
                gutter_size,
                regex: self.regex,
            }
            .render(Rect::new(area.x, y, area.width, 1), buf, handle);
            y += 1;
        }

        handle.on_mouse(area, |event| match event.kind {
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                Some(Action::Normal(NormalAction::PanVertical {
                    direction: Direction::back_if(event.kind == MouseEventKind::ScrollUp),
                    delta: ViewDelta::Number(5),
                    target_view: Some(self.view_index),
                }))
            }
            _ => None,
        });
    }
}

struct EdgeBg(bool);

impl EdgeBg {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if self.0 {
            const WIDGET_BLOCK: Block = Block::new()
                .border_style(Style::new().fg(colors::BLACK).bg(colors::GUTTER_BG))
                .style(Style::new().bg(colors::BG));

            WIDGET_BLOCK.render(area, buf);
        } else {
            const SET_LEFT_EDGE: symbols::border::Set = symbols::border::Set {
                top_left: "",
                top_right: "",
                bottom_left: "",
                bottom_right: "",
                vertical_left: "▏",
                vertical_right: "",
                horizontal_top: "",
                horizontal_bottom: "",
            };

            const LINE_WIDGET_BLOCK: Block = Block::new()
                .border_set(SET_LEFT_EDGE)
                .border_style(Style::new().fg(colors::BLACK).bg(colors::GUTTER_BG))
                .borders(Borders::LEFT)
                .style(Style::new().bg(colors::BG));

            LINE_WIDGET_BLOCK.render(area, buf);
        }
    }
}

struct FilterLineWidget<'a> {
    view_index: usize,
    inner: &'a FilterData<'a>,
}

impl FilterLineWidget<'_> {
    fn gutter_selection(line: &FilterData) -> &'static str {
        if line.ty.contains(FilterType::Origin) {
            if line.ty.contains(FilterType::OriginStart) {
                " ┌"
            } else if line.ty.contains(FilterType::OriginEnd) {
                " └"
            } else {
                " ▶"
            }
        } else if line.ty.contains(FilterType::Within) {
            " │"
        } else {
            "  "
        }
    }

    fn render(self, area: Rect, buf: &mut Buffer, handle: &mut MouseHandler) {
        let mut v = vec![
            Span::from(Self::gutter_selection(self.inner)).fg(colors::FILTER_ACCENT),
            Span::from(if self.inner.ty.contains(FilterType::Enabled) {
                " ● "
            } else {
                " ◯ "
            })
            .fg(self.inner.color),
        ];

        match self.inner.name {
            Filter::Builtin(name) => v.push(Span::raw(*name).fg(self.inner.color)),
            Filter::Literal(name, _) => {
                v.push(Span::raw("Lit ").fg(colors::TEXT_INACTIVE));
                v.push(Span::raw(name).fg(self.inner.color));
            }
            Filter::Regex(regex) => {
                v.push(Span::raw("Rgx ").fg(colors::TEXT_INACTIVE));
                v.push(Span::raw(regex.as_str()).fg(self.inner.color));
            }
        }

        if let Some(len) = self.inner.len {
            v.push(Span::from(format!(" {}", len)).fg(colors::TEXT_INACTIVE));
        }

        Paragraph::new(Line::from(v)).render(area, buf);

        handle.on_mouse(area, |event| match event.kind {
            MouseEventKind::Down(_) => Some(Action::Filter(FilterAction::ToggleFilter {
                target_view: self.view_index,
                filter_index: self.inner.index,
            })),
            _ => None,
        });
    }
}

struct ViewerLineWidget<'a> {
    view_index: usize,
    line: Option<LineData<'a>>,

    itoa_buf: &'a mut itoa::Buffer,
    show_selection: bool,
    gutter_size: Option<u16>,
    start: usize,
    regex: Option<&'a Regex>,
}

impl ViewerLineWidget<'_> {
    fn gutter_selection(line: &LineData) -> &'static str {
        if line.ty.contains(LineType::Origin) {
            if line.ty.contains(LineType::OriginStart) {
                "┌ "
            } else if line.ty.contains(LineType::OriginEnd) {
                "└"
            } else {
                "▶"
            }
        } else if line.ty.contains(LineType::Within) {
            "│"
        } else {
            ""
        }
    }

    fn split_line(&self, area: Rect) -> [Rect; 3] {
        const SPECIAL_SIZE: u16 = 3;
        let gutter_size = self.gutter_size.unwrap_or(0);
        let mut gutter_chunk = area;
        gutter_chunk.width = gutter_size;

        let mut type_chunk = area;
        type_chunk.x += gutter_size + 1;
        type_chunk.width = 1;

        let mut data_chunk = area;
        data_chunk.x += gutter_size + SPECIAL_SIZE;
        data_chunk.width = data_chunk.width.saturating_sub(gutter_size + SPECIAL_SIZE);

        [gutter_chunk, type_chunk, data_chunk]
    }

    fn render(self, area: Rect, buf: &mut Buffer, handle: &mut MouseHandler) {
        let [gutter_chunk, type_chunk, data_chunk] = self.split_line(area);

        let Some(line) = &self.line else {
            let ln = Paragraph::new("~")
                .alignment(Alignment::Right)
                .fg(colors::GUTTER_TEXT);

            ln.render(gutter_chunk, buf);
            return;
        };

        if self.gutter_size.is_some() {
            let ln_str = self.itoa_buf.format(line.line_number + 1);
            let ln = Paragraph::new(ln_str).alignment(Alignment::Right).fg(
                if line.ty.contains(LineType::Bookmarked) {
                    colors::SELECT_ACCENT
                } else {
                    colors::GUTTER_TEXT
                },
            );

            ln.render(gutter_chunk, buf);
        }

        if self.show_selection {
            Paragraph::new(Self::gutter_selection(line))
                .fg(colors::SELECT_ACCENT)
                .render(type_chunk, buf);
        }

        let mut chars = line.data.chars();
        for _ in 0..self.start {
            chars.next();
        }
        let data = chars.as_str();

        if let Some(m) = self.regex.and_then(|r| r.find(line.data.as_bytes())) {
            let start = m.start().saturating_sub(self.start);
            let end = m.end().saturating_sub(self.start);
            let spans = vec![
                Span::raw(&data[..start]),
                Span::raw(&data[start..end]).fg(colors::FILTER_ACCENT),
                Span::raw(&data[end..]),
            ];
            Paragraph::new(Line::from(spans))
        } else {
            Paragraph::new(data)
        }
        .fg(line.color)
        .render(data_chunk, buf);

        if let Some(line) = self.line {
            handle.on_mouse(area, |event| match event.kind {
                MouseEventKind::Down(_) => Some(Action::Visual(VisualAction::ToggleLine {
                    line_number: line.line_number,
                    target_view: self.view_index,
                })),
                _ => None,
            });
        }
    }
}

pub struct TabWidget<'a> {
    view_index: usize,
    name: &'a str,
    active: bool,
}

impl TabWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer, handle: &mut MouseHandler) {
        Paragraph::new(Line::from(vec![
            if self.active {
                Span::from("▍ ").fg(colors::TAB_SIDE_ACTIVE)
            } else {
                Span::from("▏ ").fg(colors::TAB_SIDE_INACTIVE)
            },
            Span::from(self.name),
        ]))
        .bg(if self.active {
            colors::TAB_ACTIVE
        } else {
            colors::TAB_INACTIVE
        })
        .fg(if self.active {
            colors::TEXT_ACTIVE
        } else {
            colors::TEXT_INACTIVE
        })
        .render(area, buf);

        handle.on_mouse(area, |event| match event.kind {
            MouseEventKind::Down(_) => Some(Action::Normal(NormalAction::SwitchActiveIndex {
                target_view: self.view_index,
            })),
            _ => None,
        });
    }
}

pub struct MultiplexerWidget<'a> {
    pub mux: &'a mut MultiplexerApp,
    pub status: &'a mut StatusApp,
    pub mode: InputMode,
    pub gutter: bool,
    pub regex: Option<&'a Regex>,
}

impl MultiplexerWidget<'_> {
    fn split_horizontal(area: Rect, len: usize) -> std::rc::Rc<[Rect]> {
        let constraints = vec![Constraint::Ratio(1, len as u32); len];
        Layout::new(ratatui::prelude::Direction::Horizontal, constraints).split(area)
    }

    pub fn split_top(area: Rect, top_height: u16) -> [Rect; 2] {
        let mut tab_chunk = area;
        tab_chunk.height = top_height;

        let mut data_chunk = area;
        data_chunk.y += top_height;
        data_chunk.height = data_chunk.height.saturating_sub(top_height);

        [tab_chunk, data_chunk]
    }

    pub fn split_bottom(area: Rect, bottom_height: u16) -> [Rect; 2] {
        let mut view_chunk = area;
        view_chunk.height = view_chunk.height.saturating_sub(bottom_height);

        let mut filter_chunk = area;
        filter_chunk.y = area.y + view_chunk.height;
        filter_chunk.height = bottom_height.min(area.height);

        [view_chunk, filter_chunk]
    }

    const FILTER_MAX_HEIGHT: u16 = 10;
    pub fn render(self, area: Rect, buf: &mut Buffer, handler: &mut MouseHandler) {
        let [mux_chunk, status_chunk] = Self::split_bottom(area, 1);

        fn fixup_chunk(fix: bool, mut chunk: Rect) -> Rect {
            if fix {
                chunk.x += 1;
                chunk.width -= 1;
            }
            chunk
        }

        if !self.mux.is_empty() {
            let active = self.mux.active();
            match self.mux.mode() {
                MultiplexerMode::Panes => {
                    for (i, (&chunk, viewer)) in Self::split_horizontal(mux_chunk, self.mux.len())
                        .iter()
                        .zip(self.mux.viewers_mut())
                        .enumerate()
                    {
                        let [tab_chunk, view_chunk] = Self::split_top(chunk, 1);
                        TabWidget {
                            view_index: i,
                            name: viewer.name(),
                            active: active == i,
                        }
                        .render(tab_chunk, buf, handler);

                        let mut viewer_chunk = view_chunk;

                        if self.mode == InputMode::Filter {
                            let [view_chunk, filter_chunk] =
                                Self::split_bottom(view_chunk, Self::FILTER_MAX_HEIGHT);
                            FilterViewerWidget {
                                view_index: i,
                                viewer,
                            }
                            .render(filter_chunk, buf, handler);
                            viewer_chunk = view_chunk;
                        }

                        ViewerWidget {
                            view_index: i,
                            show_selection: self.mode == InputMode::Visual,
                            viewer,
                            gutter: self.gutter,
                            regex: self.regex,
                        }
                        .render(
                            fixup_chunk(i != 0, viewer_chunk),
                            buf,
                            handler,
                        );
                        EdgeBg(i == 0).render(viewer_chunk, buf)
                    }
                }
                MultiplexerMode::Tabs => {
                    let [tab_chunk, view_chunk] = Self::split_top(mux_chunk, 1);

                    for (i, (&chunk, viewer)) in Self::split_horizontal(tab_chunk, self.mux.len())
                        .iter()
                        .zip(self.mux.viewers_mut())
                        .enumerate()
                    {
                        TabWidget {
                            view_index: i,
                            name: viewer.name(),
                            active: active == i,
                        }
                        .render(chunk, buf, handler);
                    }

                    let active = self.mux.active();
                    let viewer = self.mux.active_viewer_mut().unwrap();
                    let mut viewer_chunk = view_chunk;

                    if self.mode == InputMode::Filter {
                        let [view_chunk, filter_chunk] =
                            Self::split_bottom(view_chunk, Self::FILTER_MAX_HEIGHT);
                        FilterViewerWidget {
                            view_index: 0,
                            viewer,
                        }
                        .render(filter_chunk, buf, handler);
                        viewer_chunk = view_chunk;
                    }
                    ViewerWidget {
                        view_index: active,
                        show_selection: self.mode == InputMode::Visual,
                        viewer,
                        gutter: self.gutter,
                        regex: self.regex,
                    }
                    .render(viewer_chunk, buf, handler);
                    EdgeBg(true).render(viewer_chunk, buf)
                }
            }
        } else {
            const BG_BLOCK: Block = Block::new().style(Style::new().bg(colors::BG));
            BG_BLOCK.render(mux_chunk, buf);
        }

        StatusWidget {
            input_mode: self.mode,
            viewer: self.mux.active_viewer_mut().map(|v| &*v),
            message: self.status.get_message_update().as_deref(),
        }
        .render(status_chunk, buf);
    }
}
