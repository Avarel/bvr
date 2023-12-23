//! Error types and utilities.

#[derive(thiserror::Error, Debug)]
/// Represents an error that can occur in the application.
pub enum Error {
    /// An I/O error occurred.
    #[error("i/o error")]
    Io(#[from] std::io::Error),

    /// An internal error occurred.
    #[error("internal error")]
    Internal,
}

/// A specialized [Result] type for this crate's operations.
pub type Result<T, E = Error> = std::result::Result<T, E>;
