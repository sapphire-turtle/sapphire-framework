//! Crate-local errors for key-file handling.
//!
//! The remote server's error enum carries a `KeyFile(String)` variant that
//! maps into its JSON-RPC surface; the one rule this crate needs on its own is
//! the plain I/O wrapper.

use thiserror::Error;

/// Errors raised while loading, generating, or revoking API keys.
#[derive(Debug, Error)]
pub enum Error {
    /// A filesystem operation on the key file failed.
    #[error("key file I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// The key file failed to parse or save, a key could not be generated, or
    /// a revoke selector did not resolve to exactly one key.
    #[error("key file error: {0}")]
    KeyFile(String),
}

/// Convenience alias for key-store results.
pub type Result<T> = std::result::Result<T, Error>;
