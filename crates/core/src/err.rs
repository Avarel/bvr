#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("i/o error")]
    Io(#[from] std::io::Error),
    #[error("internal error")]
    Internal
}

pub type Result<T, E = Error> = std::result::Result<T, E>;