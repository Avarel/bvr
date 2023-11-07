pub mod index;
pub mod file;
mod cow_vec;

#[cfg(unix)]
use std::os::fd::AsRawFd as Mmappable;
#[cfg(windows)]
use std::os::windows::io::AsRawHandle as Mmappable;