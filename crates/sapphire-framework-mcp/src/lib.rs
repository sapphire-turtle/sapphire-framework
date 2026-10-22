//! MCP for sapphire-framework applications: the tools every app shares, and the two
//! transports that serve them.
//!
//! Every tool reaches the application the way the CLI does — over the app server's IPC — so
//! one set of tools serves both ends of an app:
//!
//! - **stdio**, on the client side: the app's `mcp` subcommand, which an agent launches as a
//!   child process. It dials the running server and never starts one (a missing server is
//!   reported per call, and the next call tries again).
//! - **HTTP**, on the server side: `/mcp` on the app server's HTTP listener, behind the
//!   external-device bearer token (`sapphire-framework-keys`), switched on in the server's
//!   config. The tools reach the server over an in-process IPC connection.
//!
//! The pieces:
//!
//! - [`ToolContext`]: what a tool holds — the IPC link to the app server, and the bridge.
//! - [`framework_tools`] and [`workspace_write_tools`]: the framework's tools, generic over
//!   the application's handler so an app adds them to its own `ToolRouter`.
//! - [`FrameworkServer`]: the handler an app gets when it adds no tools of its own.
//! - [`McpService`]: an application's handler factory, type-erased, with the stdio and HTTP
//!   transports.
//!
//! An application with tools of its own writes an `rmcp` handler that holds a
//! [`ToolContext`] and adds the framework's tools to its router:
//!
//! ```rust,ignore
//! use sapphire_framework::mcp::{self, ToolContext};
//! use sapphire_framework::mcp::rmcp::{self, handler::server::router::tool::ToolRouter, tool, tool_handler, tool_router};
//!
//! #[derive(Clone)]
//! struct LedgerTools {
//!     ctx: ToolContext,
//!     tool_router: ToolRouter<LedgerTools>,
//! }
//!
//! impl AsRef<ToolContext> for LedgerTools {
//!     fn as_ref(&self) -> &ToolContext { &self.ctx }
//! }
//!
//! #[tool_router]
//! impl LedgerTools {
//!     fn new(ctx: ToolContext) -> Self {
//!         let tool_router = Self::tool_router() + mcp::framework_tools();
//!         LedgerTools { ctx, tool_router }
//!     }
//!
//!     #[tool(description = "List the accounts")]
//!     async fn account_list(&self) -> Result<String, String> {
//!         let accounts: serde_json::Value = self.ctx.call("ledger.account_list", serde_json::json!({}))
//!             .await
//!             .map_err(|e| e.to_string())?;
//!         Ok(accounts.to_string())
//!     }
//! }
//!
//! #[tool_handler]
//! impl rmcp::ServerHandler for LedgerTools {
//!     fn get_info(&self) -> rmcp::model::ServerConfig { mcp::server_info(&self.ctx) }
//! }
//!
//! // Handed to the app server, which serves it on stdio (`mcp`) and at `/mcp`:
//! AppServer::new(&CTX, VERSION).mcp(McpService::new(LedgerTools::new));
//! ```
//!
//! The `rmcp` macros name the `rmcp` crate directly, so an application using them depends
//! on `rmcp` itself, at the version re-exported here.

#![warn(missing_docs)]

mod change;
mod context;
mod error;
mod server;
mod service;
mod tools;

pub use context::ToolContext;
pub use error::{Error, Result};
pub use server::{FrameworkServer, server_info};
pub use service::{LOOPBACK_HOSTS, McpService};
pub use tools::{framework_tools, workspace_write_tools};

/// The MCP implementation the framework builds on, so an application implements its handler
/// against the same version.
pub use rmcp;
/// The schema generator `rmcp` derives tool inputs with.
pub use schemars;

/// The [`sapphire_ipc::ClientInfo::kind`] an MCP connection reports to the app server.
pub const CLIENT_KIND: &str = "mcp";

/// The path the app server's HTTP listener serves MCP at, in every application.
pub const HTTP_PATH: &str = "/mcp";
