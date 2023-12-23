use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers, MouseEventKind};

use crate::direction::{HDirection, VDirection};

use super::{
    actions::{Action, CommandAction, Delta, FilterAction, Jump, ViewerAction},
    InputMode,
};

pub enum Keybinding {
    // The keybindings are hardcoded into the program.
    Hardcoded,
}

impl Keybinding {
    pub fn map_key(&self, input_mode: InputMode, event: Event) -> Option<Action> {
        match self {
            Self::Hardcoded => self.native_keys(input_mode, event),
            // TODO: custom keybinding feature gate
        }
    }

    fn native_keys(&self, input_mode: InputMode, event: Event) -> Option<Action> {
        if let Event::Key(key) = event {
            if key.kind != KeyEventKind::Press {
                return None;
            }
        }
        match input_mode {
            InputMode::Viewer => match event {
                Event::Mouse(mouse) => match mouse.kind {
                    MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                        Some(Action::Viewer(ViewerAction::Pan {
                            direction: VDirection::up_if(mouse.kind == MouseEventKind::ScrollUp),
                            delta: Delta::Number(2),
                        }))
                    }
                    _ => None,
                },
                Event::Key(key) => match key.code {
                    KeyCode::Char(':') => Some(Action::SwitchMode(InputMode::Command)),
                    KeyCode::Char('i') => Some(Action::SwitchMode(InputMode::Select)),
                    KeyCode::Tab => Some(Action::SwitchMode(InputMode::Filter)),
                    KeyCode::Up | KeyCode::Down => Some(Action::Viewer(ViewerAction::Pan {
                        direction: VDirection::up_if(key.code == KeyCode::Up),
                        delta: if key.modifiers.contains(KeyModifiers::SHIFT) {
                            Delta::HalfPage
                        } else {
                            Delta::Number(1)
                        },
                    })),
                    KeyCode::Home | KeyCode::End => Some(Action::Viewer(ViewerAction::Pan {
                        direction: VDirection::up_if(key.code == KeyCode::Home),
                        delta: Delta::Boundary,
                    })),
                    KeyCode::PageUp | KeyCode::PageDown => {
                        Some(Action::Viewer(ViewerAction::Pan {
                            direction: VDirection::up_if(key.code == KeyCode::PageUp),
                            delta: Delta::Page,
                        }))
                    }
                    KeyCode::Char(c @ ('u' | 'd')) => Some(Action::Viewer(ViewerAction::Pan {
                        direction: VDirection::up_if(c == 'u'),
                        delta: Delta::HalfPage,
                    })),
                    KeyCode::Left | KeyCode::Right => Some(Action::Viewer(
                        ViewerAction::SwitchActive(HDirection::left_if(key.code == KeyCode::Left)),
                    )),
                    _ => None,
                },
                _ => None,
            },
            InputMode::Filter => match event {
                Event::Key(key) => match key.code {
                    KeyCode::Char(':') => Some(Action::SwitchMode(InputMode::Command)),
                    KeyCode::Esc | KeyCode::Tab => Some(Action::SwitchMode(InputMode::Viewer)),
                    KeyCode::Char('i') => Some(Action::SwitchMode(InputMode::Select)),
                    KeyCode::Up | KeyCode::Down => Some(Action::Filter(FilterAction::Move {
                        direction: VDirection::up_if(key.code == KeyCode::Up),
                        delta: Delta::Number(1),
                    })),
                    KeyCode::Home | KeyCode::End => Some(Action::Filter(FilterAction::Move {
                        direction: VDirection::up_if(key.code == KeyCode::Home),
                        delta: Delta::Boundary,
                    })),
                    KeyCode::PageUp | KeyCode::PageDown => {
                        Some(Action::Filter(FilterAction::Move {
                            direction: VDirection::up_if(key.code == KeyCode::PageUp),
                            delta: Delta::Page,
                        }))
                    }
                    KeyCode::Char(c @ ('u' | 'd')) => Some(Action::Filter(FilterAction::Move {
                        direction: VDirection::up_if(c == 'u'),
                        delta: Delta::HalfPage,
                    })),
                    KeyCode::Left | KeyCode::Right => Some(Action::Viewer(
                        ViewerAction::SwitchActive(HDirection::left_if(key.code == KeyCode::Left)),
                    )),
                    KeyCode::Char(' ') | KeyCode::Enter => {
                        Some(Action::Filter(FilterAction::Toggle))
                    }
                    _ => None,
                },
                _ => None,
            },
            InputMode::Select => match event {
                Event::Mouse(mouse) => match mouse.kind {
                    MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                        Some(Action::Viewer(ViewerAction::Move {
                            direction: VDirection::up_if(mouse.kind == MouseEventKind::ScrollUp),
                            delta: Delta::Number(1),
                        }))
                    }
                    _ => None,
                },
                Event::Key(key) => match key.code {
                    KeyCode::Char(':') => Some(Action::SwitchMode(InputMode::Command)),
                    KeyCode::Tab => Some(Action::SwitchMode(InputMode::Filter)),
                    KeyCode::Esc => Some(Action::SwitchMode(InputMode::Viewer)),
                    KeyCode::Up | KeyCode::Down => Some(Action::Viewer(ViewerAction::Move {
                        direction: VDirection::up_if(key.code == KeyCode::Up),
                        delta: Delta::Number(1),
                    })),
                    KeyCode::Left | KeyCode::Right => Some(Action::Viewer(
                        ViewerAction::SwitchActive(HDirection::left_if(key.code == KeyCode::Left)),
                    )),
                    KeyCode::Char(' ') | KeyCode::Enter => {
                        Some(Action::Viewer(ViewerAction::ToggleLine))
                    }
                    _ => None,
                },
                _ => None,
            },
            InputMode::Command => match event {
                Event::Paste(paste) => Some(Action::Command(CommandAction::Paste(paste))),
                Event::Key(key) => match key.code {
                    KeyCode::Esc => Some(Action::SwitchMode(InputMode::Viewer)),
                    KeyCode::Tab => Some(Action::SwitchMode(InputMode::Filter)),
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
        }
    }
}
