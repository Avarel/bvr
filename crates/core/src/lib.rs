pub mod index;
pub mod buf;
pub mod search;
pub mod err;
mod cowvec;

#[cfg(unix)]
use std::os::fd::AsRawFd as Mmappable;
#[cfg(windows)]
use std::os::windows::io::AsRawHandle as Mmappable;

const SHARD_SIZE: u64 = 1 << 20;

pub use err::Result;
pub use index::inflight::InflightIndex;
pub use buf::ShardedBuffer;
pub use buf::shard::ShardStr;