pub mod buf;

pub mod collections;

pub mod err;
pub mod index;
pub mod matches;

pub use buf::{segment::SegStr, SegBuffer};
pub use err::Result;
pub use index::LineIndex;
pub use matches::LineMatches;
