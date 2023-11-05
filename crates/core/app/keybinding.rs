use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers, MouseEventKind};

use crate::common::{HDirection, VDirection};

use super::{
    actions::{Action, CommandAction, Jump, ViewerAction},
    InputMode,
};

pub enum Keybinding {
    Default
}

impl Keybinding {
    pub fn map_key(&self, input_mode: InputMode, event: Event) -> Option<Action> {
        match self {
            Self::Default => self.map_key_default(input_mode, event)
            // TODO: custom keybinding feature gate
        }
    }

    fn map_key_default(&self, input_mode: InputMode, event: Event) -> Option<Action> {
        match event {
            Event::Mouse(mouse) => match mouse.kind {
                MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                    Some(Action::Viewer(ViewerAction::Pan {
                        direction: VDirection::up_if(mouse.kind == MouseEventKind::ScrollUp),
                        delta: 2,
                    }))
                }
                _ => None,
            },
            Event::Paste(paste) => match input_mode {
                InputMode::Command => Some(Action::Command(CommandAction::Paste(paste))),
                _ => None,
            },
            Event::Key(key) => match input_mode {
                InputMode::Viewer => match key.code {
                    KeyCode::Char(':') => Some(Action::SwitchMode(InputMode::Command)),
                    KeyCode::Char('i') => Some(Action::SwitchMode(InputMode::Select)),
                    KeyCode::Esc => Some(Action::Exit),
                    KeyCode::Up | KeyCode::Down => Some(Action::Viewer(ViewerAction::Pan {
                        direction: VDirection::up_if(key.code == KeyCode::Up),
                        delta: 1,
                    })),
                    KeyCode::Right => Some(Action::Viewer(ViewerAction::SwitchActive(
                        HDirection::Right,
                    ))),
                    KeyCode::Left => {
                        Some(Action::Viewer(ViewerAction::SwitchActive(HDirection::Left)))
                    }
                    _ => None,
                },
                InputMode::Select => match key.code {
                    KeyCode::Char(':') => Some(Action::SwitchMode(InputMode::Command)),
                    KeyCode::Esc => Some(Action::SwitchMode(InputMode::Viewer)),
                    KeyCode::Up | KeyCode::Down => Some(Action::Viewer(ViewerAction::Move(
                        VDirection::up_if(key.code == KeyCode::Up),
                    ))),
                    _ => None,
                },
                InputMode::Command if key.kind == KeyEventKind::Press => match key.code {
                    KeyCode::Esc => Some(Action::SwitchMode(InputMode::Viewer)),
                    KeyCode::Enter => Some(Action::Command(CommandAction::Submit)),
                    KeyCode::Left | KeyCode::Right => Some(Action::Command(CommandAction::Move {
                        direction: HDirection::left_if(key.code == KeyCode::Left),
                        select: key.modifiers.contains(KeyModifiers::SHIFT),
                        jump: if key.modifiers.contains(KeyModifiers::ALT) {
                            Jump::Word
                        } else {
                            Jump::None
                        },
                    })),
                    KeyCode::Home | KeyCode::End => Some(Action::Command(CommandAction::Move {
                        direction: HDirection::left_if(key.code == KeyCode::Home),
                        select: key.modifiers.contains(KeyModifiers::SHIFT),
                        jump: Jump::Boundary,
                    })),
                    KeyCode::Backspace => Some(Action::Command(CommandAction::Backspace)),
                    KeyCode::Char(to_insert) => match to_insert {
                        'b' | 'f' if key.modifiers.contains(KeyModifiers::ALT) => {
                            Some(Action::Command(CommandAction::Move {
                                direction: HDirection::left_if(to_insert == 'b'),
                                select: key.modifiers.contains(KeyModifiers::SHIFT),
                                jump: Jump::Word,
                            }))
                        }
                        'a' | 'e' if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            Some(Action::Command(CommandAction::Move {
                                direction: HDirection::left_if(to_insert == 'a'),
                                select: key.modifiers.contains(KeyModifiers::SHIFT),
                                jump: Jump::Boundary,
                            }))
                        }
                        c => Some(Action::Command(CommandAction::Type(c))),
                    },
                    _ => None,
                },
                _ => None,
            },
            _ => None,
        }
    }
}
