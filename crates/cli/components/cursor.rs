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
        use std::cmp::Ordering;
        match start.cmp(&end) {
            Ordering::Less => Self::Selection(start, end, dir),
            Ordering::Equal => Self::Singleton(start),
            Ordering::Greater => Self::Selection(end, start, dir.flip()),
        }
    }
}

pub struct CursorState {
    pub state: Cursor,
}

impl CursorState {
    pub fn new() -> Self {
        Self {
            state: Cursor::Singleton(0),
        }
    }

    pub fn clamp(&mut self, bound: usize) {
        self.state = match self.state {
            Cursor::Singleton(i) => Cursor::Singleton(i.min(bound)),
            Cursor::Selection(start, end, dir) => {
                Cursor::new_range(start.min(bound), end.min(bound), dir)
            }
        }
    }

    pub fn reset(&mut self) -> Self {
        std::mem::replace(self, Self::new())
    }

    pub fn back(&mut self, select: bool, transform: impl FnOnce(usize) -> usize) {
        self.state = match self.state {
            Cursor::Singleton(i) => {
                if select && i > 0 {
                    Cursor::Selection(transform(i), i, SelectionOrigin::Left)
                } else {
                    Cursor::Singleton(transform(i))
                }
            }
            Cursor::Selection(start, end, dir) if select => match dir {
                SelectionOrigin::Right => Cursor::new_range(start, transform(end), dir),
                SelectionOrigin::Left => Cursor::new_range(transform(start), end, dir),
            },
            Cursor::Selection(start, _, _) => Cursor::Singleton(start),
        }
    }

    pub fn forward(&mut self, select: bool, transform: impl FnOnce(usize) -> usize) {
        self.state = match self.state {
            Cursor::Singleton(i) => {
                if select {
                    Cursor::new_range(i, transform(i), SelectionOrigin::Right)
                } else {
                    Cursor::Singleton(transform(i))
                }
            }
            Cursor::Selection(start, end, dir) if select => match dir {
                SelectionOrigin::Right => Cursor::new_range(start, transform(end), dir),
                SelectionOrigin::Left => Cursor::new_range(transform(start), end, dir),
            },
            Cursor::Selection(_, end, _) => Cursor::Singleton(end),
        }
    }
}
