use super::InputMode;
use crate::direction::Direction;

pub enum Action {
    #[allow(dead_code)]
    Exit,
    SwitchMode(InputMode),
    Command(CommandAction),
    Viewer(ViewerAction),
    Filter(FilterAction),
}

pub enum Delta {
    Number(u16),
    Page,
    HalfPage,
    Boundary,
}

pub enum ViewerAction {
    PanVertical {
        direction: Direction,
        delta: Delta,
        target_view: Option<usize>,
    },
    PanHorizontal {
        direction: Direction,
        delta: Delta,
        target_view: Option<usize>,
    },
    FollowOutput,
    Move {
        direction: Direction,
        select: bool,
        delta: Delta,
    },
    ToggleSelectedLine,
    ToggleLine {
        target_view: usize,
        line_number: usize,
    },
    SwitchActive(Direction),
    SwitchActiveIndex {
        target_view: usize,
    },
}

pub enum FilterAction {
    Move {
        direction: Direction,
        select: bool,
        delta: Delta,
    },
    ToggleSelectedFilter,
    RemoveSelectedFilter,
    ToggleFilter {
        target_view: usize,
        filter_index: usize,
    }
}

pub enum CommandAction {
    Move {
        direction: Direction,
        select: bool,
        jump: CommandJump,
    },
    Type(char),
    Paste(String),
    Backspace,
    Submit,
}

#[derive(Clone, Copy)]
pub enum CommandJump {
    Word,
    Boundary,
    None,
}
