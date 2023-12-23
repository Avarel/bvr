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

pub const SEARCH_COLOR_LIST: &'static [Color] = &[
    Color::Red,
    Color::Indexed(178), // Orange
    Color::Yellow,
    Color::Green,
    Color::Cyan,
    Color::Indexed(33),
    Color::Indexed(135),
    Color::Magenta,
    Color::Indexed(21),
    Color::Indexed(43),
    Color::Indexed(140),
    Color::Indexed(214),
    Color::Indexed(91),
];
