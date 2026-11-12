//! Model admission before key use and again before forwarding after a key wait.
//! Uses the token slot's in-process resume controller; called only for model completion bodies.
use super::Shared;
use super::forward::error;
use axum::http::StatusCode;
use axum::response::Response;
use std::sync::atomic::Ordering;

pub(super) async fn admit(
    shared: &Shared,
    identity: Option<&(Option<String>, Option<String>)>,
) -> Result<(), Box<Response>> {
    let Some((session, input)) = identity else {
        return Ok(());
    };
    let admitted = match shared
        .tokens
        .requests
        .admission(session.as_deref(), input.as_deref())
    {
        Ok(Some(admission)) => {
            if admission.holding() {
                shared.stats.resume_waited.fetch_add(1, Ordering::Relaxed);
            }
            admission.wait().await
        }
        Ok(None) => Ok(()),
        Err(reason) => Err(reason),
    };
    admitted.map_err(|reason| {
        shared.stats.resume_refused.fetch_add(1, Ordering::Relaxed);
        Box::new(error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "vibe_work_resume_hold",
            reason,
        ))
    })
}
