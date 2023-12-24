use crate::direction::HDirection;

use super::cursor::{Cursor, CursorState};

#[derive(Clone, Copy, Default)]
pub enum CursorJump {
    Word,
    Boundary,
    #[default]
    None,
}

#[derive(Clone, Copy)]
pub struct CursorMovement {
    delta: usize,
    select: bool,
    jump: CursorJump,
}

impl CursorMovement {
    pub const DEFAULT: Self = Self::new(false, CursorJump::None);

    pub const fn new(range_selection: bool, jump: CursorJump) -> Self {
        Self {
            delta: 1,
            select: range_selection,
            jump,
        }
    }
}

pub struct CommandApp {
    buf: String,
    cursor: CursorState,
}

impl CommandApp {
    pub fn new() -> Self {
        Self {
            buf: String::new(),
            cursor: CursorState::new(),
        }
    }

    pub fn buf(&self) -> &str {
        &self.buf
    }

    pub fn cursor(&self) -> &Cursor {
        &self.cursor.state
    }

    pub fn move_cursor(&mut self, direction: HDirection, movement: CursorMovement) {
        match direction {
            HDirection::Left => self.cursor.back(movement.select, |i| match movement.jump {
                CursorJump::Word => {
                    if self.buf[..i]
                        .chars()
                        .rev()
                        .nth(0)
                        .map(|c| c.is_whitespace())
                        .unwrap_or(false)
                    {
                        i.saturating_sub(
                            self.buf[..i]
                                .chars()
                                .rev()
                                .position(|c| c.is_alphanumeric())
                                .unwrap_or(0),
                        )
                    } else {
                        self.buf[..i].rfind(' ').map(|p| p + 1).unwrap_or(0)
                    }
                }
                CursorJump::Boundary => 0,
                CursorJump::None => i.saturating_sub(
                    self.buf[..i]
                        .chars()
                        .rev()
                        .take(movement.delta)
                        .map(|c| c.len_utf8())
                        .sum::<usize>(),
                ),
            }),
            HDirection::Right => self.cursor.forward(movement.select, self.buf.len(), |i| {
                match movement.jump {
                    CursorJump::Word => {
                        if self.buf[i..]
                            .chars()
                            .nth(0)
                            .map(|c| c.is_whitespace())
                            .unwrap_or(false)
                        {
                            i.saturating_add(
                                self.buf[i..]
                                    .chars()
                                    .position(|c| c.is_alphanumeric())
                                    .unwrap_or(usize::MAX),
                            )
                        } else {
                            self.buf[(i + 1).min(self.buf.len())..]
                                .chars()
                                .position(|c| c.is_whitespace())
                                .map(|z| z + i + 1)
                                .unwrap_or(usize::MAX)
                        }
                    }
                    CursorJump::Boundary => usize::MAX,
                    CursorJump::None => i.saturating_add(
                        self.buf[i..]
                            .chars()
                            .take(movement.delta)
                            .map(|c| c.len_utf8())
                            .sum::<usize>(),
                    ),
                }
                .min(self.buf.len())
            }),
        }
    }

    pub fn enter_char(&mut self, input: char) {
        let mut b = [0; 4];
        self.enter_str(input.encode_utf8(&mut b));
    }

    pub fn enter_str(&mut self, input: &str) {
        match self.cursor.state {
            Cursor::Singleton(i) => {
                self.buf.insert_str(i, input);
                self.move_cursor(
                    HDirection::Right,
                    CursorMovement {
                        delta: input.len(),
                        ..CursorMovement::DEFAULT
                    },
                )
            }
            Cursor::Selection(start, end, _) => {
                self.buf.replace_range(start..end, input);
                self.move_cursor(HDirection::Left, CursorMovement::DEFAULT);
                self.move_cursor(
                    HDirection::Right,
                    CursorMovement {
                        delta: input.len(),
                        ..CursorMovement::DEFAULT
                    },
                )
            }
        }
    }

    pub fn delete(&mut self) -> bool {
        match self.cursor.state {
            Cursor::Singleton(curr) => {
                if curr == 0 {
                    return !self.buf.is_empty();
                }
                self.move_cursor(HDirection::Left, CursorMovement::DEFAULT);
                let Cursor::Singleton(prev) = self.cursor.state else {
                    unreachable!()
                };
                self.buf.replace_range(prev..curr, "");
            }
            Cursor::Selection(start, end, _) => {
                self.buf.replace_range(start..end, "");
                self.move_cursor(HDirection::Left, CursorMovement::DEFAULT);
            }
        }
        true
    }

    pub fn submit(&mut self) -> String {
        self.cursor.reset();
        std::mem::take(&mut self.buf)
    }
}
