pub mod buf;

pub mod components;
pub mod cowvec;
mod cowvec2;
pub mod err;

pub use buf::{segment::SegStr, SegBuffer};
pub use components::{composite::LineComposite, index::LineIndex, matches::LineMatches};
pub use err::Result;
