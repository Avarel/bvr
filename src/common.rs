#[derive(Clone, Copy)]
pub enum HDirection {
    Left,
    Right,
}

impl HDirection {
    pub fn left_if(condition: bool) -> Self {
        if condition {
            Self::Left
        } else {
            Self::Right
        }
    }
}

#[derive(Clone, Copy)]
pub enum VDirection {
    Up,
    Down,
}

impl VDirection {
    pub fn up_if(condition: bool) -> Self {
        if condition {
            Self::Up
        } else {
            Self::Down
        }
    }
}
