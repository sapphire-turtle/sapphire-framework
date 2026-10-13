//! The handler an application gets when it adds no tools of its own.

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::tool::ToolCallContext;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, Implementation, ListToolsResult,
    PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler};

use crate::context::ToolContext;
use crate::tools::framework_tools;

/// What an application's MCP server says about itself: its name and version, that it has
/// tools, and instructions naming the workspace they act on.
///
/// An application's own handler returns this from `get_info`, or starts from it.
pub fn server_info(ctx: &ToolContext) -> ServerConfig {
    ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
        .with_server_info(Implementation::new(ctx.app(), ctx.version()))
        .with_instructions(format!(
            "Tools of {app}, a local-first application whose data is a workspace of plain-text \
             files on this host. Paths are relative to the workspace root; `workspace_show` \
             says which workspace that is. The {app} server must be running.",
            app = ctx.app()
        ))
}

/// An MCP handler with the framework's tools ([`framework_tools`]) and nothing else.
#[derive(Clone, Debug)]
pub struct FrameworkServer {
    ctx: ToolContext,
    tools: ToolRouter<FrameworkServer>,
}

impl FrameworkServer {
    /// A handler over `ctx`.
    pub fn new(ctx: ToolContext) -> Self {
        FrameworkServer {
            ctx,
            tools: framework_tools(),
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
        server_info(&self.ctx)
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
