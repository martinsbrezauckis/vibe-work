//! Shared permission service. Depends on pure ask policy; native and callback adapters call one registry.
pub(crate) mod cards;
pub(crate) mod commands;
pub(crate) mod grants;
pub(crate) mod guards;
pub(crate) mod service;
pub(crate) use cards::{callback_closed, callback_opened, callback_opened_at};
pub use commands::AskGrantsRequest;
pub use service::{AskClock, SystemAskClock};
pub(crate) use service::{Service, allowed, refused};

#[cfg(test)]
mod callback_tests;
#[cfg(test)]
mod tests;

pub(crate) mod tool_calls;

pub(crate) mod continuation;

/// A person's No on a root's own Vibe card: `cancel_turn` ends the root's turn, then the subagents that turn
/// started stop through Stop's descendant path (the whole turn stops, as for a child's No).
pub(crate) async fn root_no<T>(
    shared: &std::sync::Arc<crate::core::Shared>,
    session: &str,
    answer: impl std::future::Future<Output = Result<T, crate::CoreError>>,
) -> Result<T, crate::CoreError> {
    let root = shared.channels.canonical(session);
    let command = crate::Command::StopSession {
        session_id: root.clone(),
    };
    let stopping = crate::stops::requested(shared, &command);
    let result = answer.await;
    // The No itself was delivered: a slow subagent interrupt is logged, not shown as a failed answer.
    if let Err(error) = crate::stop_tree::descendants(shared, &root).await {
        tracing::warn!(session = %root, %error, "subagents after a No were not interrupted");
    }
    crate::stops::finished(shared, &stopping);
    result
}

/// Deliver a child's `cancel_turn` inside Stop, after the parent is quiescent and before children are stopped.
pub(crate) async fn stop_with_no<T>(
    shared: &std::sync::Arc<crate::core::Shared>,
    session: &str,
    answer: impl std::future::Future<Output = Result<T, crate::CoreError>>,
) -> Result<T, crate::CoreError> {
    let root = crate::stop_tree::owner(shared, session).unwrap_or_else(|| session.into());
    let command = crate::Command::StopSession {
        session_id: root.clone(),
    };
    let stopping = crate::stops::requested(shared, &command);
    let result = crate::stop_tree::stop_with(
        &crate::Core {
            shared: shared.clone(),
        },
        &root,
        answer,
    )
    .await;
    crate::stops::finished(shared, &stopping);
    result
}
