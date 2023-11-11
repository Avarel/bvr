pub mod index;
pub mod file;
mod cowvec;

#[cfg(unix)]
use std::os::fd::AsRawFd as Mmappable;
#[cfg(windows)]
use std::os::windows::io::AsRawHandle as Mmappable;

/// How much data of the file should each indexing task handle?
const INDEXING_VIEW_SIZE: u64 = 1 << 20;