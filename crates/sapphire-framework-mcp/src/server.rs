//! The handler an application gets when it adds no tools of its own.

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::tool::ToolCallContext;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, ListToolsResult, PaginatedRequestParams, ServerConfig,
    SubscriptionFilter, Tool,
};
use rmcp::service::{RequestContext, SubscriptionContext};
use rmcp::{ErrorData, RoleServer, ServerHandler};

use crate::context::ToolContext;
use crate::subscriptions::Subscriptions;
use crate::tools::framework_tools;

/// What an application's MCP server says about itself: its name and version, that it has
/// tools whose list a `subscriptions/listen` may follow, and instructions naming the
/// workspace they act on.
///
/// An application's own handler returns this from `get_info`, or starts from it; one that
/// offers resources on `subscriptions/listen` returns [`Subscriptions::server_info`] instead.
pub fn server_info(ctx: &ToolContext) -> ServerConfig {
    Subscriptions::new().server_info(ctx)
}

/// An MCP handler with the framework's tools ([`framework_tools`]) and nothing else; a
/// `subscriptions/listen` on it follows changes to the tool list.
#[derive(Clone, Debug)]
pub struct FrameworkServer {
    ctx: ToolContext,
    tools: ToolRouter<FrameworkServer>,
    subs: Subscriptions,
}

impl FrameworkServer {
    /// A handler over `ctx`.
    pub fn new(ctx: ToolContext) -> Self {
        FrameworkServer {
            ctx,
            tools: framework_tools(),
            subs: Subscriptions::new(),
        }
    }
}

impl AsRef<ToolContext> for FrameworkServer {
    fn as_ref(&self) -> &ToolContext {
        &self.ctx
    }
}

impl ServerHandler for FrameworkServer {
    fn get_info(&self) -> ServerConfig {
        self.subs.server_info(&self.ctx)
    }

    fn accepted_subscription_filter(
        &self,
        requested: &SubscriptionFilter,
    ) -> Option<SubscriptionFilter> {
        Some(self.subs.accept(requested))
    }

    async fn listen(&self, subscription: SubscriptionContext) -> Result<(), ErrorData> {
        crate::listen(&self.ctx, subscription).await
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        self.tools
            .call(ToolCallContext::new(self, request, context))
            .await
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(self.tools.list_all()))
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.tools.get(name).cloned()
    }
}
