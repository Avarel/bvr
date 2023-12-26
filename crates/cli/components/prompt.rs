use super::cursor::{Cursor, CursorState};
use crate::direction::Direction;

#[derive(Clone, Copy)]
pub enum PromptJump {
    Word,
    Boundary,
    Delta(usize),
}

#[derive(Clone, Copy)]
pub struct PromptMovement {
    select: bool,
    jump: PromptJump,
}

impl PromptMovement {
    pub const DEFAULT: Self = Self::new(false, PromptJump::Delta(1));

    pub const fn new(select: bool, jump: PromptJump) -> Self {
        Self { select, jump }
    }
}

pub struct PromptApp {
    buf: String,
    cursor: CursorState,
}

impl PromptApp {
    pub fn new() -> Self {
        Self {
            buf: String::new(),
            cursor: CursorState::new(),
        }
    }

    #[inline(always)]
    pub fn buf(&self) -> &str {
        &self.buf
    }

    #[inline(always)]
    pub fn cursor(&self) -> &Cursor {
        &self.cursor.state
    }

    pub fn move_cursor(&mut self, direction: Direction, movement: PromptMovement) {
        match direction {
            Direction::Back => self.cursor.back(movement.select, |i| match movement.jump {
                PromptJump::Word => {
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
                PromptJump::Boundary => 0,
                PromptJump::Delta(delta) => i.saturating_sub(
                    self.buf[..i]
                        .chars()
                        .rev()
                        .take(delta)
                        .map(|c| c.len_utf8())
                        .sum::<usize>(),
                ),
            }),
            Direction::Next => self.cursor.forward(movement.select, |i| {
                match movement.jump {
                    PromptJump::Word => {
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
                    PromptJump::Boundary => usize::MAX,
                    PromptJump::Delta(delta) => i.saturating_add(
                        self.buf[i..]
                            .chars()
                            .take(delta)
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
                    Direction::Next,
                    PromptMovement {
                        select: false,
                        jump: PromptJump::Delta(input.len()),
                    },
                )
            }
            Cursor::Selection(start, end, _) => {
                self.buf.replace_range(start..end, input);
                self.move_cursor(Direction::Back, PromptMovement::DEFAULT);
                self.move_cursor(
                    Direction::Next,
                    PromptMovement {
                        select: false,
                        jump: PromptJump::Delta(input.len()),
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
                self.move_cursor(Direction::Back, PromptMovement::DEFAULT);
                let Cursor::Singleton(prev) = self.cursor.state else {
                    unreachable!()
                };
                self.buf.replace_range(prev..curr, "");
            }
            Cursor::Selection(start, end, _) => {
                self.buf.replace_range(start..end, "");
                self.move_cursor(Direction::Back, PromptMovement::DEFAULT);
            }
        }
        true
    }

    pub fn submit(&mut self) -> String {
        self.cursor.reset();
        std::mem::take(&mut self.buf)
    }
}
