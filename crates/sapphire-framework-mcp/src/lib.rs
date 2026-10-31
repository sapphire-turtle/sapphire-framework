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
//!   API key's bearer token (`sapphire-framework-keys`), switched on in the server's
//!   config. The tools reach the server over an in-process IPC connection.
//!
//! The pieces:
//!
//! - [`ToolContext`]: what a tool holds — the IPC link to the app server, and the bridge.
//! - [`framework_tools`] and [`workspace_write_tools`]: the framework's tools, generic over
//!   the application's handler so an app adds them to its own `ToolRouter`.
//! - [`FrameworkServer`]: the handler an app gets when it adds no tools of its own.
//! - [`Subscriptions`]: what a handler offers on `subscriptions/listen`.
//! - [`McpService`]: an application's handler factory, type-erased, with the stdio and HTTP
//!   transports.
//!
//! An application with tools of its own writes an `rmcp` handler that holds a
//! [`ToolContext`] and adds the framework's tools to its router:
//!
//! ```rust,ignore
//! use sapphire_framework::mcp::{self, Subscriptions, ToolContext};
//! use sapphire_framework::mcp::rmcp::{self, handler::server::router::tool::ToolRouter, tool, tool_handler, tool_router};
//! use sapphire_framework::mcp::rmcp::model::SubscriptionFilter;
//! use sapphire_framework::mcp::rmcp::service::SubscriptionContext;
//!
//! #[derive(Clone)]
//! struct LedgerTools {
//!     ctx: ToolContext,
//!     subs: Subscriptions,
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
//!         LedgerTools { ctx, subs: Subscriptions::new(), tool_router }
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
//!     fn get_info(&self) -> rmcp::model::ServerConfig { self.subs.server_info(&self.ctx) }
//!
//!     // `subscriptions/listen`: the client hears what `ctx.notify_*` announces.
//!     fn accepted_subscription_filter(&self, requested: &SubscriptionFilter) -> Option<SubscriptionFilter> {
//!         Some(self.subs.accept(requested))
//!     }
//!     async fn listen(&self, subscription: SubscriptionContext) -> Result<(), rmcp::ErrorData> {
//!         mcp::listen(&self.ctx, subscription).await
//!     }
//! }
//!
//! // Handed to the app server, which serves it on stdio (`mcp`) and at `/mcp`:
//! AppServer::new(&CTX, VERSION).mcp(McpService::new(LedgerTools::new));
//! ```
//!
//! A handler with a need of its own — a resource each client has its own of, say — writes its
//! own `listen` instead, and may still take its capabilities and filter from
//! [`Subscriptions`].
//!
//! The `rmcp` macros name the `rmcp` crate directly, so an application using them depends
//! on `rmcp` itself, at the version re-exported here.

#![warn(missing_docs)]

mod change;
mod context;
mod error;
mod listen;
mod server;
mod service;
mod subscriptions;
mod tools;

pub use context::ToolContext;
pub use error::{Error, Result};
pub use listen::listen;
pub use server::{FrameworkServer, server_info};
pub use service::{LOOPBACK_HOSTS, McpService};
pub use subscriptions::Subscriptions;
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
