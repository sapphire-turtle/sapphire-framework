use thiserror::Error;

/// Why a tool, or a transport, could not do its work.
///
/// A tool's error is shown to the agent as the tool's result, so the messages say what to
/// do next rather than what went wrong inside.
#[derive(Debug, Error)]
pub enum Error {
    /// The application's server is not running. Nothing here starts it.
    #[error("no {0} server is running; start it with `{0} serve` or install its service")]
    NotRunning(&'static str),

    /// The connection to the server dropped. The next call dials again.
    #[error("the connection to the {0} server was lost; try again")]
    Disconnected(&'static str),

    /// The bridge could not be reached, or refused the request.
    #[error("the sapphire-bridge: {0}")]
    Bridge(sapphire_ipc::Error),

    /// The server answered the call with an error, or the IPC layer failed.
    #[error(transparent)]
    Ipc(#[from] sapphire_ipc::Error),

    /// A tool's arguments did not fit its schema.
    #[error("invalid arguments: {0}")]
    Arguments(#[from] serde_json::Error),

    /// The MCP session could not start or ended abnormally.
    #[error("MCP: {0}")]
    Mcp(String),
}

/// The crate's result type.
pub type Result<T, E = Error> = std::result::Result<T, E>;
