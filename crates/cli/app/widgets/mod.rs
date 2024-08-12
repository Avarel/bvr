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
    instance: Option<&'a Instance>,
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
            InputMode::Prompt(PromptMode::Search { escaped: false }) => {
                (colors::FILTER_ACCENT, " FILTER ")
            }
            InputMode::Prompt(PromptMode::Search { escaped: true }) => {
                (colors::FILTER_ACCENT, " FILTER (ESCAPED) ")
            }
            InputMode::Normal => (colors::NORMAL_ACCENT, " NORMAL "),
            InputMode::Visual => (colors::SELECT_ACCENT, " VISUAL "),
            InputMode::Filter => (colors::FILTER_ACCENT, " FILTER "),
        };

        let mut v = Vec::new();

        v.push(Span::from(mode_name).fg(colors::WHITE).bg(accent_color));
        v.push(Span::raw(" "));

        if let Some(instance) = self.instance {
            v.push(Span::raw(instance.name()).fg(colors::STATUS_BAR_TEXT));
        } else {
            v.push(Span::raw("Empty").fg(colors::STATUS_BAR_TEXT));
        }
        v.push(Span::raw(" │ ").fg(colors::STATUS_BAR_TEXT));

        if let Some(message) = self.message {
            v.push(Span::raw(message));
        } else if let Some(instance) = self.instance {
            let ln_cnt = instance.file().line_count();
            let ln_vis = instance.visible_line_count();
            v.push(Span::raw(format!("{} lines", ln_cnt)).fg(accent_color));
            if ln_vis < ln_cnt {
                v.push(Span::raw(format!(" ({} visible)", ln_vis)).fg(colors::STATUS_BAR_TEXT));
            }
            v.push(Span::raw(" │ ").fg(accent_color));
            v.push(Span::raw(instance.name()).fg(accent_color));
        } else {
            v.push(Span::raw(":open [file name]").fg(accent_color));
            v.push(Span::raw(" to view a file").fg(colors::STATUS_BAR_TEXT));
        }

        Paragraph::new(Line::from(v))
            .style(STATUS_BAR_STYLE)
            .render(area, buf);

        if let Some(instance) = self.instance {
            if instance.is_following_output() {
                Paragraph::new(Span::raw("Follow  ").fg(colors::STATUS_BAR_TEXT))
            } else {
                let bottom = instance.viewport().bottom();
                let ln_vis = instance.visible_line_count();
                let percentage = if ln_vis == 0 {
                    1.0
                } else {
                    bottom as f64 / ln_vis as f64
                }
                .clamp(0.0, 1.0);

                let row = instance.viewport().top();
                let col = instance.viewport().left();

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

        Paragraph::new(cmd_buf)
            .bg(colors::BG)
            .scroll((0, left as u16))
            .render(data_area, buf);

        match cursor {
            Cursor::Selection(start, end, _) => {
                let start = start.saturating_sub(left);
                let end = end.saturating_sub(left);
                let mut span_area = data_area;
                span_area.x += start as u16;
                span_area.width = (end - start) as u16;

                static HIGHLIGHT_BLOCK: OnceLock<Block> = OnceLock::new();
                HIGHLIGHT_BLOCK
                    .get_or_init(|| Block::new().style(Style::new().bg(colors::COMMAND_BAR_SELECT)))
                    .render(span_area, buf);
            }
            _ => {}
        }

        let i = match cursor {
            Cursor::Singleton(i)
            | Cursor::Selection(_, i, SelectionOrigin::Right)
            | Cursor::Selection(i, _, SelectionOrigin::Left) => {
                cmd_buf[..i.saturating_sub(left)].chars().count()
            }
        };
        *self.cursor = Some((data_area.x + i as u16, data_area.y));
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

pub struct MultiplexerPane<'a> {
    view_index: usize,
    instance: &'a mut Instance,
    show_filter_on_pane: bool,
    show_selection: bool,
    gutter: bool,
    regex: Option<&'a Regex>,
}

impl MultiplexerPane<'_> {
    const FILTER_MAX_HEIGHT: u16 = 10;

    fn render_filter_pane(
        area: &mut Rect,
        buf: &mut Buffer,
        view_index: usize,
        instance: &mut Instance,
        handler: &mut MouseHandler,
    ) {
        let [view_chunk, filter_chunk] =
            MultiplexerWidget::split_bottom(*area, Self::FILTER_MAX_HEIGHT);
        FilterViewerWidget { view_index, instance }.render(filter_chunk, buf, handler);
        *area = view_chunk;
    }

    pub fn render(self, mut area: Rect, buf: &mut Buffer, handler: &mut MouseHandler) {
        if self.show_filter_on_pane {
            Self::render_filter_pane(&mut area, buf, self.view_index, self.instance, handler);
        }

        LineViewerWidget {
            view_index: self.view_index,
            show_selection: self.show_selection,
            instance: self.instance,
            gutter: self.gutter,
            regex: self.regex,
        }
        .render(area, buf, handler);
    }
}

pub struct MultiplexerWidget<'a> {
    pub mux: &'a mut MultiplexerApp,
    pub status: &'a mut StatusApp,
    pub mode: InputMode,
    pub gutter: bool,
    pub regex: Option<&'a Regex>,
    pub linked_filters: bool,
}

impl MultiplexerWidget<'_> {
    fn split_horizontal(area: Rect, len: usize) -> std::rc::Rc<[Rect]> {
        let constraints = vec![Constraint::Ratio(1, len as u32); len];
        Layout::new(ratatui::prelude::Direction::Horizontal, constraints).split(area)
    }

    fn split_top(area: Rect, top_height: u16) -> [Rect; 2] {
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

    fn render_mux(&mut self, mut area: Rect, buf: &mut Buffer, handler: &mut MouseHandler) {
        let active = self.mux.active_index();

        let show_filter_on_pane = self.mode == InputMode::Filter && !self.linked_filters;
        let show_filter_on_mux = self.mode == InputMode::Filter && self.linked_filters;

        if show_filter_on_mux {
            MultiplexerPane::render_filter_pane(
                &mut area,
                buf,
                active,
                self.mux.active_mut().unwrap(),
                handler,
            );
        }

        let [tab_chunk, view_chunk] = Self::split_top(area, 1);
        let split_chunks = Self::split_horizontal(area, self.mux.len());

        for (view_index, (chunk, instance)) in split_chunks
            .iter()
            .map(|&chunk| tab_chunk.intersection(chunk))
            .zip(self.mux.instances_mut())
            .enumerate()
        {
            TabWidget {
                view_index,
                name: instance.name(),
                active: active == view_index,
            }
            .render(chunk, buf, handler);
        }

        match self.mux.mode() {
            MultiplexerMode::Panes => {
                for (view_index, (pane_chunk, instance)) in split_chunks
                    .iter()
                    .map(|&chunk| view_chunk.intersection(chunk))
                    .zip(self.mux.instances_mut())
                    .enumerate()
                {
                    MultiplexerPane {
                        view_index,
                        instance,
                        show_filter_on_pane,
                        show_selection: self.mode == InputMode::Visual,
                        gutter: self.gutter,
                        regex: self.regex,
                    }
                    .render(pane_chunk, buf, handler);
                }
            }
            MultiplexerMode::Tabs => {
                let instance = self.mux.active_mut().unwrap();
                let pane_chunk = view_chunk;

                MultiplexerPane {
                    view_index: active,
                    instance,
                    show_filter_on_pane,
                    show_selection: self.mode == InputMode::Visual,
                    gutter: self.gutter,
                    regex: self.regex,
                }
                .render(pane_chunk, buf, handler);
            }
        }
    }

    pub fn render(mut self, area: Rect, buf: &mut Buffer, handler: &mut MouseHandler) {
        let [mux_chunk, status_chunk] = Self::split_bottom(area, 1);

        if !self.mux.is_empty() {
            self.render_mux(mux_chunk, buf, handler);
        } else {
            const BG_BLOCK: OnceLock<Block> = OnceLock::new();
            BG_BLOCK
                .get_or_init(|| Block::new().style(Style::new().bg(colors::BG)))
                .render(mux_chunk, buf);
        }

        StatusWidget {
            input_mode: self.mode,
            instance: self.mux.active_mut().map(|v| &*v),
            message: self.status.get_message_update().as_deref(),
        }
        .render(status_chunk, buf);
    }
}
