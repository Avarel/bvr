use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::{control::ViewDelta, InputMode};
use crate::direction::Direction;

#[derive(Serialize, Deserialize)]
pub enum Action {
    Exit,
    SwitchMode(InputMode),
    Command(CommandAction),
    Normal(NormalAction),
    Visual(VisualAction),
    Filter(FilterAction),
    Config(ConfigAction),
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
    Displace {
        direction: Direction,
        delta: ViewDelta,
    },
    ToggleSelectedFilter,
    RemoveSelectedFilter,
    ToggleSpecificFilter {
        target_view: usize,
        filter_index: usize,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ConfigAction {
    Move {
        direction: Direction,
        select: bool,
        delta: ViewDelta,
    },
    LoadSelectedFilter,
    RemoveSelectedFilter,
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
        input: char,
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
