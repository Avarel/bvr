pub mod buf;

pub mod components;
pub mod err;
mod cowvec;

pub use buf::{segment::SegStr, SegBuffer};
pub use components::{composite::LineComposite, index::LineIndex, matches::LineMatches};
pub use err::Result;
