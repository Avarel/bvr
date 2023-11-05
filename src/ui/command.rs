use crate::common::HDirection;

#[derive(Clone, Copy)]
pub enum SelectionOrigin {
    Right,
    Left,
}

impl SelectionOrigin {
    pub fn flip(self) -> Self {
        match self {
            Self::Right => Self::Left,
            Self::Left => Self::Right,
        }
    }
}

#[derive(Clone, Copy)]
pub enum Cursor {
    Singleton(usize),
    Selection(usize, usize, SelectionOrigin),
}

impl Cursor {
    pub fn new_range(start: usize, end: usize, dir: SelectionOrigin) -> Self {
        if start == end {
            Self::Singleton(start)
        } else if start > end {
            Self::Selection(end, start, dir.flip())
        } else {
            Self::Selection(start, end, dir)
        }
    }
}

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
    cursor: Cursor,
}

impl CommandApp {
    pub fn new() -> Self {
        Self {
            buf: String::new(),
            cursor: Cursor::Singleton(0),
        }
    }

    pub fn buf(&self) -> &str {
        &self.buf
    }

    pub fn cursor(&self) -> &Cursor {
        &self.cursor
    }

    fn backward_index(&self, i: usize, movement: CursorMovement) -> usize {
        match movement.jump {
            CursorJump::Word => self.buf[..i].rfind(' ').unwrap_or(0),
            CursorJump::Boundary => 0,
            CursorJump::None => i.saturating_sub(movement.delta),
        }
    }

    fn forward_index(&self, i: usize, movement: CursorMovement) -> usize {
        match movement.jump {
            CursorJump::Word => self.buf[(i + 1).min(self.buf.len())..]
                .find(' ')
                .map(|z| z + i + 1)
                .unwrap_or(usize::MAX),
            CursorJump::Boundary => usize::MAX,
            CursorJump::None => i.saturating_add(movement.delta),
        }
        .min(self.buf.len())
    }

    pub fn move_cursor(&mut self, direction: HDirection, movement: CursorMovement) {
        match direction {
            HDirection::Left => {
                self.cursor = match self.cursor {
                    Cursor::Singleton(i) => {
                        if movement.select && i > 0 {
                            Cursor::Selection(
                                self.backward_index(i, movement),
                                i,
                                SelectionOrigin::Left,
                            )
                        } else {
                            Cursor::Singleton(self.backward_index(i, movement))
                        }
                    }
                    Cursor::Selection(start, end, dir) => {
                        if movement.select {
                            match dir {
                                SelectionOrigin::Right => Cursor::new_range(
                                    start,
                                    self.backward_index(end, movement),
                                    dir,
                                ),
                                SelectionOrigin::Left => Cursor::new_range(
                                    self.backward_index(start, movement),
                                    end,
                                    dir,
                                ),
                            }
                        } else {
                            Cursor::Singleton(start)
                        }
                    }
                }
            }
            HDirection::Right => {
                self.cursor = match self.cursor {
                    Cursor::Singleton(i) => {
                        if movement.select && i < self.buf.len() {
                            Cursor::new_range(
                                i,
                                self.forward_index(i, movement),
                                SelectionOrigin::Right,
                            )
                        } else {
                            Cursor::Singleton(self.forward_index(i, movement))
                        }
                    }
                    Cursor::Selection(start, end, dir) => {
                        if movement.select {
                            match dir {
                                SelectionOrigin::Right => {
                                    Cursor::new_range(start, self.forward_index(end, movement), dir)
                                }
                                SelectionOrigin::Left => {
                                    Cursor::new_range(self.forward_index(start, movement), end, dir)
                                }
                            }
                        } else {
                            Cursor::Singleton(end)
                        }
                    }
                }
            }
        }
    }


    pub fn enter_char(&mut self, input: char) {
        let mut b = [0];
        self.enter_str(input.encode_utf8(&mut b));
    }

    pub fn enter_str(&mut self, input: &str) {
        if !input.is_ascii() {
            return;
        }
        match self.cursor {
            Cursor::Singleton(i) => {
                self.buf.insert_str(i, input);
                self.move_cursor(HDirection::Right, CursorMovement {
                    delta: input.len(),
                    ..CursorMovement::DEFAULT
                })
            }
            Cursor::Selection(start, end, _) => {
                self.buf.replace_range(start..end, input);
                self.move_cursor(HDirection::Left, CursorMovement::DEFAULT);
                self.move_cursor(HDirection::Right, CursorMovement {
                    delta: input.len(),
                    ..CursorMovement::DEFAULT
                })
            }
        }
    }

    pub fn delete(&mut self) -> bool {
        match self.cursor {
            Cursor::Singleton(i) => {
                if i == 0 {
                    return self.buf.len() != 0;
                }
                self.buf.remove(i - 1);
                self.move_cursor(HDirection::Left, CursorMovement::DEFAULT)
            }
            Cursor::Selection(start, end, _) => {
                self.buf.replace_range(start..end, "");
                self.move_cursor(HDirection::Left, CursorMovement::DEFAULT);
            }
        }
        true
    }

    pub fn submit(&mut self) -> String {
        self.cursor = Cursor::Singleton(0);
        std::mem::replace(&mut self.buf, String::new())
    }
}
