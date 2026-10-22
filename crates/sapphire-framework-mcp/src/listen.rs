//! Holding a `subscriptions/listen` open, and telling it what changed.

use rmcp::ErrorData;
use rmcp::model::SubscriptionFilter;
use rmcp::service::{SubscriptionContext, SubscriptionSendError, SubscriptionSink};
use tokio::sync::broadcast::error::RecvError;

use crate::change::Change;
use crate::context::ToolContext;

/// Hold `subscription` open until the client cancels it or goes away, telling it each change
/// announced on `ctx` ([`ToolContext::notify_tool_list_changed`] and the rest) that it
/// accepted.
///
/// A handler's `ServerHandler::listen` delegates here, beside
/// [`Subscriptions::accept`](crate::Subscriptions::accept) for the filter. A subscription
/// that falls behind is told everything it accepted once: the stream cannot be resumed, and
/// a change is only a signal to read again.
///
/// rmcp acknowledges the subscription before calling here, so a change announced in between
/// is missed: a client reads what it follows once the acknowledgment arrives, as it must
/// after any reconnect.
pub async fn listen(ctx: &ToolContext, subscription: SubscriptionContext) -> Result<(), ErrorData> {
    let mut changes = ctx.changes();
    let accepted = subscription.accepted();
    let sink = subscription.sink();
    loop {
        let sent = tokio::select! {
            () = subscription.cancelled() => return Ok(()),
            received = changes.recv() => match received {
                Ok(change) if allowed(accepted, &change) => send(sink, change).await,
                Ok(_) => Ok(()),
                Err(RecvError::Lagged(_)) => {
                    let mut sent = Ok(());
                    for change in everything(accepted) {
                        sent = send(sink, change).await;
                        if sent.is_err() {
                            break;
                        }
                    }
                    sent
                }
                Err(RecvError::Closed) => return Ok(()),
            },
        };
        if let Err(err) = sent {
            tracing::debug!("a subscription ended: {err}");
            return Ok(());
        }
    }
}

async fn send(sink: &SubscriptionSink, change: Change) -> Result<(), SubscriptionSendError> {
    match change {
        Change::ToolListChanged => sink.notify_tool_list_changed().await,
        Change::ResourceListChanged => sink.notify_resource_list_changed().await,
        Change::ResourceUpdated(uri) => sink.notify_resource_updated(uri).await,
    }
}

/// Whether a subscription that accepted `accepted` is told of `change`.
fn allowed(accepted: &SubscriptionFilter, change: &Change) -> bool {
    match change {
        Change::ToolListChanged => accepted.tools_list_changed == Some(true),
        Change::ResourceListChanged => accepted.resources_list_changed == Some(true),
        Change::ResourceUpdated(uri) => accepted
            .resource_subscriptions
            .as_ref()
            .is_some_and(|uris| uris.contains(uri)),
    }
}

/// Every change a subscription that accepted `accepted` can be told of: what a lagging one
/// hears.
fn everything(accepted: &SubscriptionFilter) -> Vec<Change> {
    let lists = [
        (accepted.tools_list_changed, Change::ToolListChanged),
        (accepted.resources_list_changed, Change::ResourceListChanged),
    ];
    lists
        .into_iter()
        .filter(|(on, _)| *on == Some(true))
        .map(|(_, change)| change)
        .chain(
            accepted
                .resource_subscriptions
                .iter()
                .flatten()
                .cloned()
                .map(Change::ResourceUpdated),
        )
        .collect()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use rmcp::model::{
        ProtocolVersion, ServerConfig, ServerNotification, SubscriptionFilter,
    };
    use rmcp::service::{RunningService, SubscriptionContext};
    use rmcp::{
        ClientLifecycleMode, ClientServiceExt as _, ErrorData, RoleClient, ServerHandler,
        ServiceExt as _,
    };
    use sapphire_ipc::Endpoint;

    use super::*;
    use crate::{FrameworkServer, Subscriptions};

    const WAIT: Duration = Duration::from_secs(5);
    const QUIET: Duration = Duration::from_millis(200);

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
            listen(&self.ctx, subscription).await
        }
    }

    // rmcp acknowledges a subscription before `listen` subscribes to the changes, so a test
    // waits for the listener before announcing anything.

    /// A 2026-07-28 client of `handler`, over an in-memory pipe: the transport `serve_stdio`
    /// runs on stdin and stdout.
    async fn connect<S: ServerHandler + Send + Sync + 'static>(
        handler: S,
    ) -> RunningService<RoleClient, ()> {
        let (client, server) = tokio::io::duplex(64 * 1024);
        tokio::spawn(async move {
            if let Ok(running) = handler.serve(server).await {
                let _ = running.waiting().await;
            }
        });
        tokio::time::timeout(
            WAIT,
            ().serve_with_lifecycle(
                client,
                ClientLifecycleMode::Discover {
                    preferred_versions: vec![ProtocolVersion::LATEST],
                },
            ),
        )
        .await
        .expect("connect in time")
        .expect("connect")
    }

    fn uris(filter: &SubscriptionFilter) -> Vec<String> {
        filter.resource_subscriptions.clone().unwrap_or_default()
    }

    #[test]
    fn allowed_follows_the_accepted_filter() {
        let accepted = SubscriptionFilter::builder()
            .tools_list_changed()
            .resource_subscription("x://a")
            .build();
        assert!(allowed(&accepted, &Change::ToolListChanged));
        assert!(!allowed(&accepted, &Change::ResourceListChanged));
        assert!(allowed(&accepted, &Change::ResourceUpdated("x://a".into())));
        assert!(!allowed(&accepted, &Change::ResourceUpdated("x://b".into())));
        assert!(!allowed(&SubscriptionFilter::new(), &Change::ToolListChanged));
    }

    #[test]
    fn lag_sends_everything_accepted_once() {
        let accepted = SubscriptionFilter::builder()
            .tools_list_changed()
            .resource_subscriptions(["x://a", "x://b"])
            .build();
        assert_eq!(
            everything(&accepted),
            vec![
                Change::ToolListChanged,
                Change::ResourceUpdated("x://a".into()),
                Change::ResourceUpdated("x://b".into()),
            ]
        );
    }

    #[tokio::test]
    async fn the_framework_server_tells_a_listener_its_tools_changed() {
        let ctx = ctx();
        let client = connect(FrameworkServer::new(ctx.clone())).await;
        let mut subscription = client
            .listen(SubscriptionFilter::builder().tools_list_changed().build())
            .await
            .unwrap();
        assert_eq!(subscription.acknowledged().tools_list_changed, Some(true));
        wait_for_listeners(&ctx, 1).await;
        ctx.notify_tool_list_changed();
        let next = tokio::time::timeout(WAIT, subscription.next()).await.unwrap();
        assert!(matches!(
            next,
            Ok(Some(ServerNotification::ToolListChangedNotification(_)))
        ));
    }

    #[tokio::test]
    async fn an_offered_resource_is_accepted_and_others_dropped() {
        let ctx = ctx();
        let client = connect(Offering::new(ctx.clone())).await;
        let mut subscription = client
            .listen(
                SubscriptionFilter::builder()
                    .resource_subscriptions(["x://a", "x://b"])
                    .build(),
            )
            .await
            .unwrap();
        assert_eq!(uris(subscription.acknowledged()), ["x://a"]);
        wait_for_listeners(&ctx, 1).await;
        ctx.notify_resource_updated("x://b");
        ctx.notify_resource_updated("x://a");
        let next = tokio::time::timeout(WAIT, subscription.next()).await.unwrap();
        match next {
            Ok(Some(ServerNotification::ResourceUpdatedNotification(n))) => {
                assert_eq!(n.params.uri, "x://a")
            }
            other => panic!("expected the x://a update, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_change_before_listening_is_not_replayed() {
        let ctx = ctx();
        let client = connect(FrameworkServer::new(ctx.clone())).await;
        ctx.notify_tool_list_changed();
        let mut subscription = client
            .listen(SubscriptionFilter::builder().tools_list_changed().build())
            .await
            .unwrap();
        wait_for_listeners(&ctx, 1).await;
        assert!(
            tokio::time::timeout(QUIET, subscription.next())
                .await
                .is_err(),
            "a change from before the listen arrived"
        );
    }

    #[tokio::test]
    async fn a_listen_for_nothing_offered_is_acknowledged_empty() {
        let ctx = ctx();
        let client = connect(FrameworkServer::new(ctx.clone())).await;
        let subscription = client
            .listen(
                SubscriptionFilter::builder()
                    .resource_subscription("x://b")
                    .build(),
            )
            .await
            .unwrap();
        assert_eq!(subscription.acknowledged(), &SubscriptionFilter::new());
        let tools = tokio::time::timeout(WAIT, client.list_all_tools())
            .await
            .unwrap()
            .unwrap();
        assert!(!tools.is_empty());
    }

    #[tokio::test]
    async fn cancelling_the_subscription_ends_listen() {
        let ctx = ctx();
        let client = connect(FrameworkServer::new(ctx.clone())).await;
        let mut subscription = client
            .listen(SubscriptionFilter::builder().tools_list_changed().build())
            .await
            .unwrap();
        wait_for_listeners(&ctx, 1).await;
        subscription.cancel().await.unwrap();
        wait_for_listeners(&ctx, 0).await;
    }

    async fn wait_for_listeners(ctx: &ToolContext, n: usize) {
        tokio::time::timeout(WAIT, async {
            while ctx.listeners() != n {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("{} listeners, not {n}", ctx.listeners()));
    }
}
