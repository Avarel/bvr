pub mod index;
pub mod buf;
pub mod matches;
pub mod composite;

pub mod err;
pub mod cowvec;

mod inflight_tool;

pub use err::Result;
pub use buf::SegBuffer;
pub use buf::segment::SegStr;

pub use index::inflight::InflightIndex;
pub use matches::inflight::InflightSearch;
pub use composite::inflight::InflightComposite;