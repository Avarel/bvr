pub mod buf;

pub mod components;
pub mod cowvec;
pub mod err;

pub use buf::segment::SegStr;
pub use buf::SegBuffer;
pub use err::Result;

pub use components::composite::InflightComposite;
pub use components::index::InflightIndex;
pub use components::matches::InflightMatches;
