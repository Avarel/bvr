use super::{
    actions::{Action, NormalAction},
    mouse::MouseHandler,
    InputMode, PromptMode,
};
use crate::{
    app::widgets::{filters::FilterViewerWidget, viewer::LineViewerWidget},
    colors,
    components::{
        cursor::{Cursor, SelectionOrigin},
        instance::Instance,
        mux::{MultiplexerApp, MultiplexerMode},
        prompt::PromptApp,
        status::StatusApp,
    },
};
use crossterm::event::MouseEventKind;
use ratatui::{prelude::*, widgets::*};
use regex::bytes::Regex;
use std::sync::OnceLock;

mod filters;
mod viewer;

pub struct StatusWidget<'a> {
    input_mode: InputMode,
    viewer: Option<&'a Instance>,
    message: Option<&'a str>,
}

impl<'a> Widget for StatusWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        const STATUS_BAR_STYLE: Style = Style::new()
            .fg(colors::STATUS_BAR_TEXT)
            .bg(colors::STATUS_BAR);

        let (accent_color, mode_name) = match self.input_mode {
            InputMode::Prompt(PromptMode::Command) => (colors::COMMAND_ACCENT, " COMMAND "),
            InputMode::Prompt(PromptMode::Shell { .. }) => (colors::SHELL_ACCENT, " SHELL "),
            InputMode::Prompt(PromptMode::Search { regex: true }) => {
                (colors::FILTER_ACCENT, " FILTER ")
            }
            InputMode::Prompt(PromptMode::Search { regex: false }) => {
                (colors::FILTER_ACCENT, " FILTER (ESCAPED) ")
            }
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

impl PromptWidget<'_> {
    pub fn split_prompt(area: Rect) -> [Rect; 2] {
        let mut indicator_chunk = area;
        indicator_chunk.width = 1;

        let mut data_chunk = area;
        data_chunk.width -= 1;
        data_chunk.x += 1;

        [indicator_chunk, data_chunk]
    }

    pub fn render(self, area: Rect, buf: &mut Buffer) {
        let InputMode::Prompt(mode) = self.mode else {
            static WIDGET_BLOCK: OnceLock<Block> = OnceLock::new();
            WIDGET_BLOCK
                .get_or_init(|| Block::new().style(Style::new().bg(colors::BG)))
                .render(area, buf);
            return;
        };

        let [indicator_area, data_area] = Self::split_prompt(area);

        match mode {
            PromptMode::Command => Span::raw(":").fg(colors::COMMAND_ACCENT),
            PromptMode::Search { .. } => Span::raw("/").fg(colors::FILTER_ACCENT),
            PromptMode::Shell { pipe: true } => Span::raw("|").fg(colors::SHELL_ACCENT),
            PromptMode::Shell { pipe: false } => Span::raw("!").fg(colors::SHELL_ACCENT),
        }
        .render(indicator_area, buf);

        let cursor = self.inner.cursor();
        let left = self.inner.viewport().left();
        let cmd_buf = self.inner.view_and_update(usize::from(area.width));

        let input = Paragraph::new(Line::from(match cursor {
            Cursor::Singleton(_) => {
                vec![Span::raw(cmd_buf)]
            }
            Cursor::Selection(start, end, _) => vec![
                Span::raw(&cmd_buf[..start]),
                Span::raw(&cmd_buf[start..end]).bg(colors::COMMAND_BAR_SELECT),
                Span::raw(&cmd_buf[end..]),
            ],
        }))
        .bg(colors::BG)
        .scroll((0, left as u16));

        let i = match cursor {
            Cursor::Singleton(i)
            | Cursor::Selection(_, i, SelectionOrigin::Right)
            | Cursor::Selection(i, _, SelectionOrigin::Left) => {
                cmd_buf[..i.saturating_sub(left)].chars().count()
            }
        };
        *self.cursor = Some((data_area.x + i as u16, data_area.y));

        input.render(data_area, buf);
    }
}

struct EdgeBg(bool);

impl EdgeBg {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if self.0 {
            static WIDGET_BLOCK: OnceLock<Block> = OnceLock::new();
            WIDGET_BLOCK
                .get_or_init(|| {
                    Block::new()
                        .border_style(Style::new().fg(colors::BLACK).bg(colors::GUTTER_BG))
                        .style(Style::new().bg(colors::BG))
                })
                .render(area, buf);
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

            static LINE_WIDGET_BLOCK: OnceLock<Block> = OnceLock::new();
            LINE_WIDGET_BLOCK
                .get_or_init(|| {
                    Block::new()
                        .border_set(SET_LEFT_EDGE)
                        .border_style(Style::new().fg(colors::BLACK).bg(colors::GUTTER_BG))
                        .borders(Borders::LEFT)
                        .style(Style::new().bg(colors::BG))
                })
                .render(area, buf);
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

                        EdgeBg(i == 0).render(viewer_chunk, buf);

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

                        LineViewerWidget {
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

                    EdgeBg(true).render(viewer_chunk, buf);

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
                    LineViewerWidget {
                        view_index: active,
                        show_selection: self.mode == InputMode::Visual,
                        viewer,
                        gutter: self.gutter,
                        regex: self.regex,
                    }
                    .render(viewer_chunk, buf, handler);
                }
            }
        } else {
            const BG_BLOCK: OnceLock<Block> = OnceLock::new();
            BG_BLOCK
                .get_or_init(|| Block::new().style(Style::new().bg(colors::BG)))
                .render(mux_chunk, buf);
        }

        StatusWidget {
            input_mode: self.mode,
            viewer: self.mux.active_viewer_mut().map(|v| &*v),
            message: self.status.get_message_update().as_deref(),
        }
        .render(status_chunk, buf);
    }
}
