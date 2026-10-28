//! Sends an admitted call, bounding session opens and cleaning up failed compaction.
//! Used by dispatch after applying chat choices; preserves the existing Stop and channel ordering.
use serde_json::Value;
use vw_gate::{ConfirmationToken, Reply};

use crate::core::Core;
use crate::dispatch_compaction::Compaction;
use crate::error::{CoreError, ErrorCode};

pub(crate) struct Sending<'a> {
    pub(crate) method: &'a str,
    pub(crate) params: &'a Value,
    pub(crate) token: Option<&'a ConfirmationToken>,
    pub(crate) stop: Option<u64>,
    pub(crate) opening: bool,
    pub(crate) compaction: Option<Compaction>,
    pub(crate) selected: Option<crate::model_choices::Choice>,
}

impl Core {
    pub(crate) async fn send_call(&self, sending: Sending<'_>) -> Result<Reply, CoreError> {
        let Sending {
            method,
            params,
            token,
            stop,
            opening,
            compaction,
            selected,
        } = sending;
        if method == "config/model/write" && token.is_none() {
            return Box::pin(crate::model_choices::pick(&self.shared, params, selected)).await;
        }
        let call = crate::worktrees_deleted::call(&self.shared, method, params, token, stop);
        if let Some(id) = params["sessionId"].as_str().filter(|_| opening) {
            let mut reply = None;
            self.bounded_open(id, async {
                reply = Some(call.await.map_err(|e| CoreError::from_gate(method, &e))?);
                Ok(())
            })
            .await?;
            return reply
                .ok_or_else(|| CoreError::new(ErrorCode::Internal, "resume without a reply"));
        }
        match call.await {
            Ok(reply) => Ok(reply),
            Err(error) => Err(crate::dispatch_compaction::failed(
                &self.shared,
                method,
                params,
                compaction,
                &error,
            )
            .await),
        }
    }
}
