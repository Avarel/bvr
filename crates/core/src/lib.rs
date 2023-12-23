pub mod buf;
pub mod composite;
pub mod index;
pub mod matches;

pub mod cowvec;
pub mod err;

mod inflight_tool;

pub use buf::segment::SegStr;
pub use buf::SegBuffer;
pub use err::Result;

pub use composite::inflight::InflightComposite;
pub use index::inflight::InflightIndex;
pub use matches::inflight::InflightSearch;
