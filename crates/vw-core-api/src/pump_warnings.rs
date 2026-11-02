//! Applies channel notifications to the session store, retaining the source of sessionless warnings.
//! Called by the pump; depends on RPC frames, channel ownership and the session notice projection.

use vw_rpc::Notification;
use vw_session::{Applied, SessionStore};

pub(super) fn apply(
    store: &SessionStore,
    mut notification: Notification,
    channel_session: Option<String>,
) -> Applied {
    // Vibe's ServerWarningParams carries no session id; the attached channel owns the warning.
    if notification.method == "warning"
        && notification
            .params
            .get("sessionId")
            .and_then(serde_json::Value::as_str)
            .is_none()
        && let Some(session) = channel_session
        && let Some(params) = notification.params.as_object_mut()
    {
        params.insert("sessionId".into(), session.into());
    }
    store.apply(vw_session::Notification::new(
        notification.method,
        notification.params,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use vw_session::Change;

    #[test]
    fn real_warning_frames_keep_the_session_of_their_channel() {
        let params = json!({"warning": {"code": "integrations_unavailable", "message": "Calendar unavailable."}});
        let applied = apply(
            &SessionStore::new(),
            Notification {
                method: "warning".into(),
                params,
            },
            Some("chat".into()),
        );
        assert!(
            matches!(&applied.changes[..], [Change::Notice { session_id: Some(id), method, .. }] if id == "chat" && method == "warning"),
            "{applied:?}"
        );
    }

    #[test]
    fn lobby_warnings_and_explicit_child_sessions_keep_their_scope() {
        for (params, channel, expected) in [
            (
                json!({"warning": {"message": "Catalogue fallback"}}),
                None,
                None,
            ),
            (
                json!({"sessionId": "child", "warning": {"message": "Child warning"}}),
                Some("parent".into()),
                Some("child"),
            ),
        ] {
            let applied = apply(
                &SessionStore::new(),
                Notification {
                    method: "warning".into(),
                    params,
                },
                channel,
            );
            assert!(
                matches!(&applied.changes[..], [Change::Notice { session_id, .. }] if session_id.as_deref() == expected),
                "{applied:?}"
            );
        }
    }
}
