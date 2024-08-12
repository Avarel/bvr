use ratatui::style::Color;

pub const WHITE: Color = Color::Indexed(255);
pub const BLACK: Color = Color::Indexed(16);
pub const BG: Color = Color::Reset;

pub const TEXT_ACTIVE: Color = Color::Indexed(253);
pub const TEXT_INACTIVE: Color = Color::Indexed(238);

pub const GUTTER_BG: Color = BG;
pub const GUTTER_TEXT: Color = Color::Indexed(241);

pub const TAB_INACTIVE: Color = Color::Indexed(235);
pub const TAB_ACTIVE: Color = Color::Indexed(239);
pub const TAB_SIDE_ACTIVE: Color = Color::Indexed(39);
pub const TAB_SIDE_INACTIVE: Color = Color::Black;

pub const STATUS_BAR: Color = Color::Indexed(235);
pub const STATUS_BAR_TEXT: Color = Color::Indexed(246);

pub const COMMAND_BAR_SELECT: Color = Color::Indexed(69);

pub const NORMAL_ACCENT: Color = Color::Indexed(75);
pub const COMMAND_ACCENT: Color = Color::Indexed(48);
pub const SELECT_ACCENT: Color = Color::Indexed(170);
pub const FILTER_ACCENT: Color = Color::Indexed(178);

pub const SHELL_ACCENT: Color = Color::Indexed(161);

pub struct ColorSelector {
    hue: f64
}

impl ColorSelector {
    pub const DEFAULT: Self = Self {
        hue: 0.0
    };

    pub fn reset(&mut self) {
        *self = Self::DEFAULT
    }

    pub fn peek_color(&self) -> Color {
        Color::from_hsl(self.hue, 80.0, 50.0)
    }

    pub fn next_color(&mut self) -> Color {
        let color = self.peek_color();
        self.hue += 208.3;
        self.hue %= 360.0;
        color
    }
}
