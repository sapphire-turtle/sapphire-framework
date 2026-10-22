//! The handler an application gets when it adds no tools of its own.

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::tool::ToolCallContext;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, Implementation, ListToolsResult,
    PaginatedRequestParams, ServerCapabilities, ServerConfig, SubscriptionFilter, Tool,
};
use rmcp::service::{RequestContext, SubscriptionContext};
use rmcp::{ErrorData, RoleServer, ServerHandler};

use crate::context::ToolContext;
use crate::subscriptions::Subscriptions;
use crate::tools::framework_tools;

/// What an application's MCP server says about itself: its name and version, that it has
/// tools, and instructions naming the workspace they act on.
///
/// An application's own handler returns this from `get_info`, or starts from it. It promises
/// no `subscriptions/listen`: a handler that answers one returns
/// [`Subscriptions::server_info`] instead, which says what it may be told.
pub fn server_info(ctx: &ToolContext) -> ServerConfig {
    describe(ctx, ServerCapabilities::builder().enable_tools().build())
}

/// [`server_info`] with `capabilities`.
pub(crate) fn describe(ctx: &ToolContext, capabilities: ServerCapabilities) -> ServerConfig {
    ServerConfig::new(capabilities)
        .with_server_info(Implementation::new(ctx.app(), ctx.version()))
        .with_instructions(format!(
            "Tools of {app}, a local-first application whose data is a workspace of plain-text \
             files on this host. Paths are relative to the workspace root; `workspace_show` \
             says which workspace that is. The {app} server must be running.",
            app = ctx.app()
        ))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> ToolContext {
        let endpoint = sapphire_ipc::Endpoint::in_dir("sapphire-test", std::env::temp_dir());
        ToolContext::dial("sapphire-test", "0.0.0", None, endpoint)
    }

    /// An application whose handler returns `server_info` and does not answer
    /// `subscriptions/listen` must not promise tool list changes it never sends.
    #[test]
    fn server_info_promises_no_list_changes() {
        let tools = server_info(&ctx())
            .capabilities
            .tools
            .expect("tools advertised");
        assert_ne!(tools.list_changed, Some(true));
    }

    #[test]
    fn the_framework_server_promises_tool_list_changes() {
        let info = FrameworkServer::new(ctx()).get_info();
        assert_eq!(
            info.capabilities.tools.and_then(|t| t.list_changed),
            Some(true)
        );
    }
}
