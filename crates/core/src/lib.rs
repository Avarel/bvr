pub mod buf;

pub mod components;
pub mod cowvec;
pub mod err;

pub use buf::{segment::SegStr, SegBuffer};
pub use components::{
    composite::InflightComposite, index::InflightIndex, matches::InflightMatches,
};
pub use err::Result;
