use crate::direction::{HDirection, VDirection};

use super::InputMode;

pub enum Action {
    Exit,
    SwitchMode(InputMode),
    Command(CommandAction),
    Viewer(ViewerAction),
}

pub enum Delta {
    Number(u16),
    Page,
    HalfPage,
    Boundary
}

pub enum ViewerAction {
    Pan {
        direction: VDirection,
        delta: Delta
    },
    Move {
        direction: VDirection,
        delta: Delta
    },
    ToggleLine,
    SwitchActive(HDirection),
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