pub mod index;
pub mod buf;
pub mod search;
pub mod err;
mod cowvec;

pub use err::Result;
pub use index::inflight::InflightIndex;
pub use buf::SegBuffer;
pub use buf::segment::SegStr;