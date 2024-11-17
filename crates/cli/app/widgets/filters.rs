use super::super::{
    actions::{Action, FilterAction},
    mouse::MouseHandler,
};
use crate::{
    colors,
    components::{cursor::Cursor, filters::Mask, instance::Instance},
};
use bitflags::bitflags;
use crossterm::event::MouseEventKind;
use ratatui::{prelude::*, widgets::*};
use std::sync::OnceLock;

pub struct FilterViewerWidget<'a> {
    pub(super) view_index: usize,
    pub(super) instance: &'a mut Instance,
}

impl FilterViewerWidget<'_> {
    pub fn render(self, area: Rect, buf: &mut Buffer, handle: &mut MouseHandler) {
        static WIDGET_BLOCK: OnceLock<Block> = OnceLock::new();
        WIDGET_BLOCK
            .get_or_init(|| Block::new().style(Style::new().bg(colors::STATUS_BAR)))
            .render(area, buf);

        let cursor_state = self.instance.compositor_mut().cursor().state();

        let view = self
            .instance
            .compositor_mut()
            .update_and_filter_view(area.height as usize);

        (area.y..area.bottom())
            .zip(view)
            .for_each(|(y, (index, filter))| {
                FilterLineWidget {
                    view_index: self.view_index,
                    index,
                    name: filter.mask(),
                    color: filter.color(),
                    len: filter.len(),
                    ty: match cursor_state {
                        Cursor::Singleton(i) => {
                            if index == i {
                                FilterType::Origin
                            } else {
                                FilterType::None
                            }
                        }
                        Cursor::Selection(start, end, _) => {
                            if !(start..=end).contains(&index) {
                                FilterType::None
                            } else if index == start {
                                FilterType::Origin | FilterType::OriginStart
                            } else if index == end {
                                FilterType::Origin | FilterType::OriginEnd
                            } else {
                                FilterType::Within
                            }
                        }
                    } | if filter.is_enabled() {
                        FilterType::Enabled
                    } else {
                        FilterType::None
                    },
                }
                .render(Rect::new(area.x, y, area.width, 1), buf, handle);
            });
    }
}

struct FilterLineWidget<'a> {
    view_index: usize,
    index: usize,
    name: &'a Mask,
    color: Color,
    len: Option<usize>,
    ty: FilterType,
}

bitflags! {
    struct FilterType: u8 {
        const None = 0;
        const Enabled = 1 << 0;
        const Origin = 1 << 1;
        const OriginStart = 1 << 2;
        const OriginEnd = 1 << 3;
        const Within = 1 << 4;
    }
}

impl FilterLineWidget<'_> {
    fn gutter_selection(&self) -> &'static str {
        if self.ty.contains(FilterType::Origin) {
            if self.ty.contains(FilterType::OriginStart) {
                " ┌"
            } else if self.ty.contains(FilterType::OriginEnd) {
                " └"
            } else {
                " ▶"
            }
        } else if self.ty.contains(FilterType::Within) {
            " │"
        } else {
            "  "
        }
    }

    pub fn render(self, area: Rect, buf: &mut Buffer, handle: &mut MouseHandler) {
        let mut v = vec![
            Span::from(self.gutter_selection()).fg(colors::FILTER_ACCENT),
            Span::from(if self.ty.contains(FilterType::Enabled) {
                " ● "
            } else {
                " ◯ "
            })
            .fg(self.color),
        ];

        v.push(Span::raw(self.name.name()).fg(self.color));

        if let Some(len) = self.len {
            v.push(Span::from(format!(" {}", len)).fg(colors::TEXT_INACTIVE));
        }

        Paragraph::new(Line::from(v)).render(area, buf);

        handle.on_mouse(area, |event| match event.kind {
            MouseEventKind::Down(_) => Some(Action::Filter(FilterAction::ToggleFilter {
                target_view: self.view_index,
                filter_index: self.index,
            })),
            _ => None,
        });
    }
}
