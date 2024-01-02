use super::cursor::{Cursor, CursorState};
use crate::direction::Direction;

#[derive(Clone, Copy)]
pub enum PromptDelta {
    Number(usize),
    Word,
    Boundary,
}

#[derive(Clone, Copy)]
pub struct PromptMovement {
    select: bool,
    delta: PromptDelta,
}

impl PromptMovement {
    pub const DEFAULT: Self = Self::new(false, PromptDelta::Number(1));

    pub const fn new(select: bool, delta: PromptDelta) -> Self {
        Self { select, delta }
    }
}

pub struct PromptApp {
    history: Vec<String>,
    index: usize,
    buf: String,
    cursor: CursorState,
}

impl PromptApp {
    pub fn new() -> Self {
        Self {
            history: Vec::new(),
            index: 0,
            buf: String::new(),
            cursor: CursorState::new(),
        }
    }

    #[inline(always)]
    pub fn buf(&self) -> &str {
        if self.index < self.history.len() {
            &self.history[self.index]
        } else {
            &self.buf
        }
    }

    #[inline(always)]
    pub fn cursor(&self) -> Cursor {
        self.cursor.state()
    }

    pub fn move_cursor(&mut self, direction: Direction, movement: PromptMovement) {
        let buf = if self.index < self.history.len() {
            &self.history[self.index]
        } else {
            &self.buf
        };
        match direction {
            Direction::Back => self.cursor.back(movement.select, |i| match movement.delta {
                PromptDelta::Word => {
                    if buf[..i]
                        .chars()
                        .rev()
                        .nth(0)
                        .map(|c| c.is_whitespace())
                        .unwrap_or(false)
                    {
                        i.saturating_sub(
                            buf[..i]
                                .chars()
                                .rev()
                                .position(|c| c.is_alphanumeric())
                                .unwrap_or(0),
                        )
                    } else {
                        buf[..i].rfind(' ').map(|p| p + 1).unwrap_or(0)
                    }
                }
                PromptDelta::Boundary => 0,
                PromptDelta::Number(delta) => i.saturating_sub(
                    buf[..i]
                        .chars()
                        .rev()
                        .take(delta)
                        .map(|c| c.len_utf8())
                        .sum::<usize>(),
                ),
            }),
            Direction::Next => self.cursor.forward(movement.select, |i| {
                match movement.delta {
                    PromptDelta::Word => {
                        if buf[i..]
                            .chars()
                            .nth(0)
                            .map(|c| c.is_whitespace())
                            .unwrap_or(false)
                        {
                            i.saturating_add(
                                buf[i..]
                                    .chars()
                                    .position(|c| c.is_alphanumeric())
                                    .unwrap_or(usize::MAX),
                            )
                        } else {
                            buf[(i + 1).min(buf.len())..]
                                .chars()
                                .position(|c| c.is_whitespace())
                                .map(|z| z + i + 1)
                                .unwrap_or(usize::MAX)
                        }
                    }
                    PromptDelta::Boundary => usize::MAX,
                    PromptDelta::Number(delta) => i.saturating_add(
                        buf[i..]
                            .chars()
                            .take(delta)
                            .map(|c| c.len_utf8())
                            .sum::<usize>(),
                    ),
                }
                .min(buf.len())
            }),
        }
    }

    pub fn enter_char(&mut self, input: char) {
        let mut b = [0; 4];
        self.enter_str(input.encode_utf8(&mut b));
    }

    pub fn enter_str(&mut self, input: &str) {
        if self.index < self.history.len() {
            self.buf = self.history[self.index].clone();
            self.index = self.history.len();
        }

        match self.cursor.state() {
            Cursor::Singleton(i) => {
                self.buf.insert_str(i, input);
                self.move_cursor(
                    Direction::Next,
                    PromptMovement {
                        select: false,
                        delta: PromptDelta::Number(input.len()),
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
                        delta: PromptDelta::Number(input.len()),
                    },
                )
            }
        }
    }

    pub fn delete(&mut self) -> bool {
        if self.index < self.history.len() {
            self.buf = self.history[self.index].clone();
            self.index = self.history.len();
        }

        match self.cursor.state() {
            Cursor::Singleton(curr) => {
                if curr == 0 {
                    return !self.buf.is_empty();
                }
                self.move_cursor(Direction::Back, PromptMovement::DEFAULT);
                let Cursor::Singleton(prev) = self.cursor.state() else {
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

    #[allow(dead_code)]
    pub fn replace_last_word(&mut self, word: &str) {
        let buf = if self.index < self.history.len() {
            &mut self.history[self.index]
        } else {
            &mut self.buf
        };
        if let Some(i) = buf.rfind(' ') {
            buf.replace_range(i + 1.., word);
        } else {
            *buf = word.to_owned();
        }
    }

    pub fn backward(&mut self) {
        self.index = self.index.saturating_sub(1);
        self.cursor.place(self.buf().len());
    }

    pub fn forward(&mut self) {
        self.index = self.index.saturating_add(1).min(self.history.len());
        self.cursor.place(self.buf().len());
    }

    pub fn submit(&mut self) -> String {
        let output = self.take();
        if self.history.last() != Some(&output) {
            self.history.push(output.clone());
            self.index = self.history.len();
        }
        output
    }

    pub fn take(&mut self) -> String {
        self.cursor.reset();
        if self.index < self.history.len() {
            let output = self.history.remove(self.index);
            self.index = self.history.len();
            output
        } else {
            std::mem::take(&mut self.buf)
        }
    }
}
