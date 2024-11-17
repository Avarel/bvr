use serde::{Deserialize, Serialize};

#[derive(PartialEq, Clone, Copy, Serialize, Deserialize)]
#[serde(tag = "mode")]
pub enum InputMode {
    Prompt(PromptMode),
    Normal,
    Visual,
    Filter,
    Config,
}

impl InputMode {
    pub fn is_prompt_search(&self) -> bool {
        matches!(self, InputMode::Prompt(PromptMode::Search { .. }))
    }
}

#[derive(PartialEq, Clone, Copy, Serialize, Deserialize)]
#[serde(tag = "prompt")]
pub enum PromptMode {
    Command,
    Shell { pipe: bool },
    Search { escaped: bool, edit: bool },
}

#[derive(Serialize, Deserialize, Clone, Copy)]
#[serde(tag = "delta")]
pub enum ViewDelta {
    Number(u16),
    Page,
    HalfPage,
    Boundary,
    Match,
}
