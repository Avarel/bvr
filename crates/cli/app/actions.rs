use super::{InputMode, ViewDelta};
use crate::direction::Direction;

pub enum Action {
    Exit,
    SwitchMode(InputMode),
    Command(CommandAction),
    Normal(NormalAction),
    Visual(VisualAction),
    Filter(FilterAction),
}

pub enum NormalAction {
    PanVertical {
        direction: Direction,
        delta: ViewDelta,
        target_view: Option<usize>,
    },
    PanHorizontal {
        direction: Direction,
        delta: ViewDelta,
        target_view: Option<usize>,
    },
    FollowOutput,
    SwitchActive(Direction),
    SwitchActiveIndex {
        target_view: usize,
    },
}

pub enum VisualAction {
    Move {
        direction: Direction,
        select: bool,
        delta: ViewDelta,
    },
    ToggleSelectedLine,
    ToggleLine {
        target_view: usize,
        line_number: usize,
    },
}

pub enum FilterAction {
    Move {
        direction: Direction,
        select: bool,
        delta: ViewDelta,
    },
    ToggleSelectedFilter,
    RemoveSelectedFilter,
    ToggleFilter {
        target_view: usize,
        filter_index: usize,
    },
}

pub enum CommandAction {
    Move {
        direction: Direction,
        select: bool,
        jump: CommandJump,
    },
    History {
        direction: Direction,
    },
    Type(char),
    Paste(String),
    Backspace,
    Submit,
    Complete,
}

#[derive(Clone, Copy)]
pub enum CommandJump {
    Word,
    Boundary,
    None,
}
