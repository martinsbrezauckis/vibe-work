//! The plugin-approval sweep that runs before Vibe may load the plugins root.
//! Depends on the parent's private approval state; called by `Channels::request` and `detached_runtime::config`.
use super::Channels;

impl Channels {
    /// Moves every unapproved plugin folder off before a request lets Vibe load the plugins root: the requests
    /// `loads_plugins` names, and the lobby `config/read` that stands in for a detached chat's own config read
    /// (`detached_runtime::config`). Serialized with approval changes.
    ///
    /// # Errors
    /// The sweep's own failure (a folder that could not be moved): the caller must not forward its request.
    pub(crate) async fn sweep_plugins(&self) -> Result<(), crate::error::CoreError> {
        let _held = self.plugin_approvals.mutations.lock().await;
        // Fresh pinned-Python identity batches run on the blocking pool.
        let approvals = self.plugin_approvals.clone();
        tokio::task::spawn_blocking(move || approvals.sweep())
            .await
            .map_err(|join| {
                crate::error::CoreError::new(crate::ErrorCode::Internal, join.to_string())
            })?
    }
}
