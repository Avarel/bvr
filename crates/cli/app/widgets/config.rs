use super::super::mouse::MouseHandler;
use crate::{
    colors,
    components::{config::filter::FilterConfigApp, cursor::Cursor},
};
use bitflags::bitflags;
use ratatui::{prelude::*, widgets::*};
use std::sync::OnceLock;

pub struct ConfigViewerWidget<'a> {
    pub(super) app: &'a mut FilterConfigApp,
}

impl ConfigViewerWidget<'_> {
    fn split_left(area: Rect, left_width: u16) -> [Rect; 2] {
        let mut left_chunk = area;
        left_chunk.width = left_width;

        let mut right_chunk = area;
        right_chunk.x += left_width;
        right_chunk.width = right_chunk.width.saturating_sub(left_width);

        [left_chunk, right_chunk]
    }

    fn split_half(area: Rect) -> [Rect; 2] {
        Self::split_left(area, area.width / 2)
    }

    pub fn render(self, area: Rect, buf: &mut Buffer, handle: &mut MouseHandler) {
        let [left_chunk, right_chunk] = Self::split_half(area);
        {
            static WIDGET_BLOCK: OnceLock<Block> = OnceLock::new();
            WIDGET_BLOCK
                .get_or_init(|| Block::new().style(Style::new().bg(colors::STATUS_BAR)))
                .render(left_chunk, buf);

            let cursor_state = self.app.cursor().state();

            let view = self.app.update_and_filter_view(left_chunk.height as usize);

            (left_chunk.y..left_chunk.bottom())
                .zip(view)
                .for_each(|(y, (index, filter))| {
                    ConfigLineWidget {
                        name: filter.name(),
                        ty: match cursor_state {
                            Cursor::Singleton(i) => {
                                if index == i {
                                    ConfigType::Origin
                                } else {
                                    ConfigType::None
                                }
                            }
                            Cursor::Selection(start, end, _) => {
                                if !(start..=end).contains(&index) {
                                    ConfigType::None
                                } else if index == start {
                                    ConfigType::Origin | ConfigType::OriginStart
                                } else if index == end {
                                    ConfigType::Origin | ConfigType::OriginEnd
                                } else {
                                    ConfigType::Within
                                }
                            }
                        },
                    }
                    .render(
                        Rect::new(left_chunk.x, y, left_chunk.width, 1),
                        buf,
                        handle,
                    );
                });
        }
        if let Some(filter) = self.app.selected_filter() {
            static WIDGET_BLOCK: OnceLock<Block> = OnceLock::new();
            WIDGET_BLOCK
                .get_or_init(|| Block::new().style(Style::new().bg(colors::BLACK)))
                .render(right_chunk, buf);

            (right_chunk.y..right_chunk.bottom())
                .zip(filter.filters())
                .for_each(|(y, filter)| {
                    FilterLineWidget {
                        color: filter.color(),
                        name: filter.name(),
                        enabled: filter.is_enabled(),
                    }
                    .render(
                        Rect::new(right_chunk.x, y, right_chunk.width, 1),
                        buf,
                        handle,
                    );
                });
        }
    }
}

struct ConfigLineWidget<'a> {
    name: Option<&'a str>,
    ty: ConfigType,
}

bitflags! {
    struct ConfigType: u8 {
        const None = 0;
        const Origin = 1 << 1;
        const OriginStart = 1 << 2;
        const OriginEnd = 1 << 3;
        const Within = 1 << 4;
    }
}

impl ConfigLineWidget<'_> {
    fn gutter_selection(&self) -> &'static str {
        if self.ty.contains(ConfigType::Origin) {
            if self.ty.contains(ConfigType::OriginStart) {
                " ┌ "
            } else if self.ty.contains(ConfigType::OriginEnd) {
                " └ "
            } else {
                " ▶ "
            }
        } else if self.ty.contains(ConfigType::Within) {
            " │ "
        } else {
            " - "
        }
    }

    pub fn render(self, area: Rect, buf: &mut Buffer, _: &mut MouseHandler) {
        let mut v = vec![Span::from(self.gutter_selection()).fg(colors::CONFIG_ACCENT)];

        v.push(Span::raw(self.name.unwrap_or("Untitled Filter Set")).fg(colors::WHITE));

        Paragraph::new(Line::from(v)).render(area, buf);
    }
}

struct FilterLineWidget<'a> {
    color: Color,
    name: &'a str,
    enabled: bool,
}

impl FilterLineWidget<'_> {
    pub fn render(self, area: Rect, buf: &mut Buffer, _: &mut MouseHandler) {
        let spans = vec![
            Span::from(if self.enabled { " ● " } else { " ◯ " }).fg(self.color),
            Span::raw(self.name).fg(self.color),
        ];
        Paragraph::new(Line::from(spans)).render(area, buf);
    }
}
