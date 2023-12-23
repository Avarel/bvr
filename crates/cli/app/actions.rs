use crate::direction::{HDirection, VDirection};

use super::InputMode;

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
    Pan { direction: VDirection, delta: Delta, target_view: Option<usize> },
    MoveSelect { direction: VDirection, delta: Delta },
    ToggleSelectedLine,
    ToggleLine {
        target_view: usize,
        line_number: usize,
    },
    SwitchActive(HDirection),
    SwitchActiveIndex(usize),
}

pub enum FilterAction {
    MoveSelect { direction: VDirection, delta: Delta },
    ToggleSelectedFilter,
    RemoveSelectedFilter,
}

pub enum CommandAction {
    Move {
        direction: HDirection,
        select: bool,
        jump: Jump,
    },
    Type(char),
    Paste(String),
    Backspace,
    Submit,
}

#[derive(Clone, Copy)]
pub enum Jump {
    Word,
    Boundary,
    None,
}
