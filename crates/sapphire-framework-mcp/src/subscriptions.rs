//! What a handler offers on `subscriptions/listen`.

use rmcp::model::{Implementation, ServerCapabilities, ServerConfig, SubscriptionFilter};

use crate::context::ToolContext;

/// What a handler offers on `subscriptions/listen`: changes to its tool list, always, and
/// updates to the resources added here.
///
/// One value gives both what the handler advertises ([`capabilities`](Self::capabilities),
/// [`server_info`](Self::server_info)) and what it accepts ([`accept`](Self::accept)). rmcp
/// acknowledges only what both allow, so the two must agree.
#[derive(Clone, Debug, Default)]
pub struct Subscriptions {
    resources: Vec<String>,
}

impl Subscriptions {
    /// Changes to the tool list, and nothing else.
    pub fn new() -> Self {
        Subscriptions::default()
    }

    /// Offer updates to the resource at `uri` too.
    pub fn resource(mut self, uri: impl Into<String>) -> Self {
        self.resources.push(uri.into());
        self
    }

    /// The capabilities to advertise: tools with `listChanged`, and, when any resource is
    /// offered, resources with `subscribe` and `listChanged`.
    pub fn capabilities(&self) -> ServerCapabilities {
        let tools = ServerCapabilities::builder()
            .enable_tools()
            .enable_tool_list_changed();
        if self.resources.is_empty() {
            tools.build()
        } else {
            tools
                .enable_resources()
                .enable_resources_subscribe()
                .enable_resources_list_changed()
                .build()
        }
    }

    /// What of `requested` to acknowledge: the parts that are offered.
    pub fn accept(&self, requested: &SubscriptionFilter) -> SubscriptionFilter {
        let mut offer = SubscriptionFilter::builder().tools_list_changed();
        if !self.resources.is_empty() {
            offer = offer
                .resources_list_changed()
                .resource_subscriptions(self.resources.iter().cloned());
        }
        requested.intersection(&offer.build())
    }

    /// [`server_info`](crate::server_info)'s name, version and instructions, with these
    /// capabilities.
    pub fn server_info(&self, ctx: &ToolContext) -> ServerConfig {
        ServerConfig::new(self.capabilities())
            .with_server_info(Implementation::new(ctx.app(), ctx.version()))
            .with_instructions(format!(
                "Tools of {app}, a local-first application whose data is a workspace of \
                 plain-text files on this host. Paths are relative to the workspace root; \
                 `workspace_show` says which workspace that is. The {app} server must be \
                 running.",
                app = ctx.app()
            ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn everything_requested() -> SubscriptionFilter {
        SubscriptionFilter::builder()
            .tools_list_changed()
            .resources_list_changed()
            .resource_subscriptions(["x://a", "x://b"])
            .build()
    }

    #[test]
    fn the_default_offers_tool_list_changes_only() {
        let caps = Subscriptions::new().capabilities();
        assert_eq!(caps.tools.and_then(|t| t.list_changed), Some(true));
        assert!(caps.resources.is_none());
    }

    #[test]
    fn a_resource_adds_subscribe_and_list_changed() {
        let caps = Subscriptions::new().resource("x://a").capabilities();
        let resources = caps.resources.expect("resources advertised");
        assert_eq!(resources.subscribe, Some(true));
        assert_eq!(resources.list_changed, Some(true));
    }

    #[test]
    fn accept_keeps_only_what_is_offered() {
        let accepted = Subscriptions::new()
            .resource("x://a")
            .accept(&everything_requested());
        let expected = SubscriptionFilter::builder()
            .tools_list_changed()
            .resources_list_changed()
            .resource_subscription("x://a")
            .build();
        assert_eq!(accepted, expected);
    }

    #[test]
    fn accept_without_resources_drops_resource_requests() {
        let accepted = Subscriptions::new().accept(&everything_requested());
        assert_eq!(
            accepted,
            SubscriptionFilter::builder().tools_list_changed().build()
        );
    }

    #[test]
    fn accept_grants_nothing_unrequested() {
        let accepted = Subscriptions::new()
            .resource("x://a")
            .accept(&SubscriptionFilter::new());
        assert_eq!(accepted, SubscriptionFilter::new());
    }

    #[test]
    fn accept_agrees_with_what_rmcp_lets_through() {
        let subs = Subscriptions::new().resource("x://a");
        let requested = everything_requested();
        let accepted = subs.accept(&requested);
        let advertised = requested.supported_by(&subs.capabilities());
        assert_eq!(
            requested.intersection(&accepted).intersection(&advertised),
            accepted
        );
    }
}
