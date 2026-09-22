use thiserror::Error;

/// Errors surfaced by a [`WorkspaceBackend`](crate::WorkspaceBackend).
#[derive(Debug, Error)]
pub enum Error {
    /// The underlying local workspace failed.
    #[error(transparent)]
    Workspace(#[from] sapphire_workspace::Error),

    /// The IPC layer failed.
    #[error(transparent)]
    Ipc(#[from] sapphire_ipc::Error),

    /// A blocking task panicked or was cancelled.
    #[error("backend task failed: {0}")]
    Join(String),

    /// The operation is not available on this backend.
    #[error("operation not supported by this backend: {0}")]
    Unsupported(&'static str),

    /// A workspace registry entry or selection was invalid (e.g. an unknown
    /// workspace id).
    #[error("invalid workspace configuration: {0}")]
    InvalidWorkspace(String),
}

impl From<tokio::task::JoinError> for Error {
    fn from(e: tokio::task::JoinError) -> Self {
        Error::Join(e.to_string())
    }
}

/// Convenience alias for backend results.
pub type Result<T> = std::result::Result<T, Error>;
