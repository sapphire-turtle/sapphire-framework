//! Errors raised while installing a service.

use thiserror::Error;

/// Errors raised while installing a service.
#[derive(Debug, Error)]
pub enum Error {
    /// A file operation failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),

    /// The requested kind of installation is not supported on this platform.
    #[error("system-wide installation is supported on Linux only; {0}")]
    Unsupported(String),

    /// A user the install needs to name could not be determined.
    ///
    /// Carries what the caller should have provided (`--run-as <user>`) and why it matters.
    #[error("{0}")]
    MissingUser(String),

    /// The OS service manager refused something.
    #[error("the service manager failed: {0}")]
    Manager(String),

    /// The install request contradicts itself.
    #[error("{0}")]
    Config(String),
}

/// Convenience alias for service-install results.
pub type Result<T> = std::result::Result<T, Error>;
