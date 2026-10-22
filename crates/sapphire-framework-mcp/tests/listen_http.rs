//! `subscriptions/listen` over `/mcp`, the way an agent on the network reaches it.
//!
//! The HTTP transport builds a handler per request, so these pin that a subscription opened
//! by one request hears what the context announces, whichever request's handler it is.

use std::time::Duration;

use rmcp::model::{ProtocolVersion, ServerConfig, ServerNotification, SubscriptionFilter};
use rmcp::service::{RunningService, SubscriptionContext};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::{ClientLifecycleMode, ClientServiceExt as _, ErrorData, RoleClient, ServerHandler};
use sapphire_framework_mcp::{self as mcp, McpService, Subscriptions, ToolContext};
use sapphire_ipc::Endpoint;
use tokio_util::sync::CancellationToken;

const WAIT: Duration = Duration::from_secs(5);

fn ctx() -> ToolContext {
    let endpoint = Endpoint::in_dir("sapphire-test", std::env::temp_dir());
    ToolContext::dial("sapphire-test", "0.0.0", None, endpoint)
}

/// A handler offering updates to `x://a`.
#[derive(Clone)]
struct Offering {
    ctx: ToolContext,
    subs: Subscriptions,
}

impl Offering {
    fn new(ctx: ToolContext) -> Self {
        Offering {
            ctx,
            subs: Subscriptions::new().resource("x://a"),
        }
    }
}

impl ServerHandler for Offering {
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
        mcp::listen(&self.ctx, subscription).await
    }
}

/// Serve `service`'s `/mcp` over `ctx` on a loopback port; its URL.
async fn serve(service: McpService, ctx: &ToolContext) -> String {
    let router = service.http_router(ctx.clone(), &[], CancellationToken::new());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, router).await });
    format!("http://127.0.0.1:{port}{}", mcp::HTTP_PATH)
}

/// A 2026-07-28 client of `url`.
async fn connect(url: &str) -> RunningService<RoleClient, ()> {
    tokio::time::timeout(
        WAIT,
        ().serve_with_lifecycle(
            StreamableHttpClientTransport::from_uri(url.to_owned()),
            ClientLifecycleMode::Discover {
                preferred_versions: vec![ProtocolVersion::LATEST],
            },
        ),
    )
    .await
    .expect("connect in time")
    .expect("connect")
}

fn tools_list_changed() -> SubscriptionFilter {
    SubscriptionFilter::builder().tools_list_changed().build()
}

/// Announce with `announce` until `subscription` hears something; what it heard.
///
/// rmcp acknowledges a subscription before the handler's `listen` subscribes to the changes,
/// and from outside the crate there is no telling when it has, so the first announcement may
/// go unheard. Repeating it is what a change is for: a signal to read again.
async fn hear(
    subscription: &mut rmcp::service::Subscription,
    announce: impl Fn(),
) -> ServerNotification {
    tokio::time::timeout(WAIT, async {
        loop {
            announce();
            if let Ok(heard) =
                tokio::time::timeout(Duration::from_millis(100), subscription.next()).await
            {
                return heard
                    .expect("the subscription")
                    .expect("an open subscription");
            }
        }
    })
    .await
    .expect("a notification in time")
}

#[tokio::test(flavor = "multi_thread")]
async fn http_listen_hears_tool_list_changes() {
    let ctx = ctx();
    let client = connect(&serve(McpService::framework(), &ctx).await).await;
    let mut subscription = client.listen(tools_list_changed()).await.unwrap();
    assert_eq!(subscription.acknowledged().tools_list_changed, Some(true));
    assert!(matches!(
        hear(&mut subscription, || ctx.notify_tool_list_changed()).await,
        ServerNotification::ToolListChangedNotification(_)
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn http_listen_hears_an_offered_resource() {
    let ctx = ctx();
    let client = connect(&serve(McpService::new(Offering::new), &ctx).await).await;
    let mut subscription = client
        .listen(
            SubscriptionFilter::builder()
                .resource_subscriptions(["x://a", "x://b"])
                .build(),
        )
        .await
        .unwrap();
    assert_eq!(
        subscription
            .acknowledged()
            .resource_subscriptions
            .as_deref(),
        Some(&["x://a".to_owned()][..])
    );
    match hear(&mut subscription, || ctx.notify_resource_updated("x://a")).await {
        ServerNotification::ResourceUpdatedNotification(n) => assert_eq!(n.params.uri, "x://a"),
        other => panic!("expected the x://a update, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn http_shares_changes_across_requests() {
    let ctx = ctx();
    let url = serve(McpService::framework(), &ctx).await;
    let (first, second) = (connect(&url).await, connect(&url).await);
    let mut a = first.listen(tools_list_changed()).await.unwrap();
    let mut b = second.listen(tools_list_changed()).await.unwrap();
    for subscription in [&mut a, &mut b] {
        assert!(matches!(
            hear(subscription, || ctx.notify_tool_list_changed()).await,
            ServerNotification::ToolListChangedNotification(_)
        ));
    }
}
