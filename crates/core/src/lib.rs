pub mod buf;

#[allow(dead_code)]
pub(crate) mod collections;
mod cowvec;

pub mod err;
pub mod index;
pub mod matches;

pub use buf::{segment::SegStr, SegBuffer};
pub use err::Result;
pub use index::LineIndex;
pub use matches::LineMatches;
