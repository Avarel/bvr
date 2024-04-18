use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::{InputMode, ViewDelta};
use crate::direction::Direction;

#[derive(Serialize, Deserialize)]
pub enum Action {
    Exit,
    SwitchMode(InputMode),
    Command(CommandAction),
    Normal(NormalAction),
    Visual(VisualAction),
    Filter(FilterAction),
    ExportFile(PathBuf),
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type")]
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

#[derive(Serialize, Deserialize)]
#[serde(tag = "type")]
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

#[derive(Serialize, Deserialize)]
#[serde(tag = "type")]
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

#[derive(Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum CommandAction {
    Move {
        direction: Direction,
        select: bool,
        jump: CommandJump,
    },
    History {
        direction: Direction,
    },
    Type {
        input: char
    },
    Paste {
        input: String,
    },
    Backspace,
    Submit,
    Complete,
}

#[derive(Serialize, Deserialize, Clone, Copy)]
#[serde(tag = "type")]
pub enum CommandJump {
    Word,
    Boundary,
    None,
}
