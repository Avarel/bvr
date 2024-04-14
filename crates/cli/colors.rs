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

pub const VIEWER_ACCENT: Color = Color::Indexed(75);
pub const COMMAND_ACCENT: Color = Color::Indexed(48);
pub const SELECT_ACCENT: Color = Color::Indexed(170);
pub const FILTER_ACCENT: Color = Color::Indexed(178);

pub const SHELL_ACCENT: Color = Color::Indexed(161);

const SEARCH_COLOR_LIST: &[Color] = &[
    Color::Red,          // red
    Color::Indexed(33),  // blue
    Color::Green,        // green
    Color::Indexed(135), // purple
    Color::Indexed(178), // gold
    Color::Cyan,         //cyan
    Color::Magenta,      // magenta
    Color::Yellow,       // yellow
    Color::Indexed(21),  // indigo
    Color::Indexed(43),  // torquoise
    Color::Indexed(140),
    Color::Indexed(214),
    Color::Indexed(91),
];

pub struct ColorSelector {
    color_list: &'static [Color],
    index: usize,
}

impl ColorSelector {
    pub const DEFAULT: Self = Self {
        color_list: SEARCH_COLOR_LIST,
        index: 0,
    };

    pub fn peek_color(&self) -> Color {
        self.color_list[self.index]
    }

    pub fn next_color(&mut self) -> Color {
        let color = self.color_list[self.index];
        self.index = (self.index + 1) % self.color_list.len();
        color
    }
}
