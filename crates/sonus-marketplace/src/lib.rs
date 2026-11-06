//! Reviewed GitHub release index and transactional, WASM-only local installation.
//! Checksums bind bytes to a reviewed index; they are not publisher signatures.
mod catalog;
mod client;
mod developer;
mod package;
mod plan;
mod progress;
mod store;

pub use catalog::*;
pub use client::*;
pub use developer::*;
pub use package::*;
pub use plan::*;
pub use progress::*;
pub use store::*;

/// File extension (without the dot) of an installable extension package.
pub const PACKAGE_EXTENSION: &str = "exaudio";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Invalid(String),
    #[error("installation blocked: {0}")]
    Blocked(String),
    #[error("operation cancelled before activation")]
    Cancelled,
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Http(#[from] reqwest::Error),
    #[error(transparent)]
    Zip(#[from] zip::result::ZipError),
}
pub type Result<T, E = Error> = std::result::Result<T, E>;

pub(crate) fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}

#[cfg(test)]
mod tests;
