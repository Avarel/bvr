use serde::{Deserialize, Serialize};

#[derive(Clone, Copy)]
#[derive(Serialize, Deserialize)]
pub enum Direction {
    Back,
    Next,
}

impl Direction {
    #[inline(always)]
    pub fn back_if(condition: bool) -> Self {
        if condition {
            Self::Back
        } else {
            Self::Next
        }
    }
}
