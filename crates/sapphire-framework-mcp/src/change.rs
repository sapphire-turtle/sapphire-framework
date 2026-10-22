//! What `ToolContext::notify_*` announces to the subscriptions listening on it.

/// How many changes may queue for a subscription before it lags: it then hears everything
/// it accepted once, since which changes it lost is unknown.
pub(crate) const CHANGE_CAPACITY: usize = 64;

/// A change a subscription may be told of.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Change {
    ToolListChanged,
    ResourceListChanged,
    ResourceUpdated(String),
}
