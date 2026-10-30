//! Which channel a request goes to: pure rules over the method name and params, used by
//! `channels` (the gate's transport) and `dispatch` (which calls need the session on a channel).
//!
//! Depends on: `serde_json`. Called by: `channels`, `dispatch`.
//! Invariant: no I/O, no state; every rule is tested here.

use serde_json::Value;

/// Methods that need the session on a channel: Vibe answers them only for the session a channel has
/// attached (`_handler.py` calls `_require_attached` / `_require_session` for them;
/// `turn/start` for a session from a channel that does not hold it answers `not_found`). Before one
/// of these, a session without a channel is resumed on a new one (`Core::open_session` in
/// `recovery.rs`, after the gate has checked the call itself). Everything else that names a session
/// works from the lobby for a stored session (read, rename, pin, archive, markAsSeen) or is not
/// used yet. Session-channel methods include `turn/start`, `callback/result`, and
/// `session/agent/update`; the others follow Vibe's handler requirements.
const NEEDS_CHANNEL: &[&str] = &[
    "loops/list",
    "loops/create",
    "loops/delete",
    "loops/clear",
    "skills/list",
    "skills/installed",
    "skills/catalog",
    "skills/detail",
    "skills/versions",
    "skills/setEnabled",
    "skills/import",
    "skills/setLatest",
    // An offered update pins the offered version; Vibe's skills service requires the attached session for every
    // mutation and for the updates read (`_skills_service.py`: `_begin_mutation`, `require_session`).
    "skills/setVersion",
    "skills/setAlias",
    "skills/convertLocal",
    "skills/updates",
    "skills/remove",
    "turn/start",
    "turn/steer",
    "turn/interrupt",
    "callback/result",
    "session/turn/enqueue",
    "session/turn/queue/read",
    "session/turn/queue/remove",
    "session/turn/queue/replace",
    "session/turn/queue/resume",
    "session/shellCommand",
    "session/agent/update",
    "session/context/inject",
    "session/rewind",
    "session/compact",
    "session/history/clear",
    "session/fork",
    "config/model/write",
    // `account/read` from the lobby for a session whose channel had closed
    // answered `conflict`, "Start, resume, or continue a session before using this method"): the
    // settings screens read and write config and the account through a session, so a session without a
    // channel is reopened first. `config/reload` is the `/reload` command's call.
    "config/write",
    "config/reload",
    "config/fields/read",
    // The settings screen reads and writes the proxy and certificate settings through a session too.
    "config/proxy/read",
    "config/proxy/write",
    "account/read",
    "identity/read",
    // Vibe answers a connector sign-in only for the session its channel holds (`_target` in
    // `connector_catalog.py`; from the lobby it is `not_found`).
    "connector_catalog/auth/request",
    "connector_catalog/connect",
];

/// The session `method` needs on a channel, when it is one of [`NEEDS_CHANNEL`] (a fork names its
/// source as `sourceSessionId`, everything else as `sessionId`).
pub(crate) fn needs_channel<'a>(method: &str, params: &'a Value) -> Option<&'a str> {
    if !NEEDS_CHANNEL.contains(&method) {
        return None;
    }
    let key = if method == "session/fork" {
        "sourceSessionId"
    } else {
        "sessionId"
    };
    params.get(key).and_then(Value::as_str)
}

/// Whether a successful `method` may hand the channel's session to the id its result names (the result's
/// `state.session.id` differs from the session it was sent for). With `--multiplex`, a fork
/// with `attach: true` (the default), a
/// rewind with `inplace: false` and `session/history/clear` do move the channel to the new id; a fork with
/// `attach: false` returns the fork's state but the channel keeps the SOURCE attached (moving the binding
/// left the source stuck and the fork `not_found`).
pub(crate) fn may_hand_off(method: &str, params: &Value) -> bool {
    !(method == "session/fork" && params.get("attach").and_then(Value::as_bool) == Some(false))
}

/// Where one request goes.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Route {
    Lobby,
    New,
    Resume(String),
    Delete(String),
    Session(String),
}

pub(crate) fn route(method: &str, params: &Value) -> Route {
    let text = |key: &str| params.get(key).and_then(Value::as_str).map(str::to_owned);
    match method {
        "session/start" | "session/continue" => Route::New,
        "session/resume" => text("sessionId").map_or(Route::Lobby, Route::Resume),
        "session/delete" => text("sessionId").map_or(Route::Lobby, Route::Delete),
        "session/fork" => text("sourceSessionId").map_or(Route::Lobby, Route::Session),
        _ => text("sessionId").map_or(Route::Lobby, Route::Session),
    }
}

// Pinned Vibe builds full plugin contexts for lifecycle requests and stored-session config reads.
// A config read without a session uses the host's plugin-free context; reload explicitly rescans.
pub(crate) fn loads_plugins(method: &str, params: &Value) -> bool {
    matches!(
        method,
        "session/start" | "session/resume" | "session/continue" | "session/fork" | "plugin/reload"
    ) || (method == "config/read" && params.get("sessionId").and_then(Value::as_str).is_some())
}

/// The id of the session a result carries, if any.
pub(crate) fn result_session(result: &Value) -> Option<&str> {
    result.pointer("/state/session/id").and_then(Value::as_str)
}

#[cfg(test)]
mod tests {
    use super::{Route, needs_channel, route};
    use serde_json::json;

    #[test]
    fn routes_follow_the_session_named_by_the_request() {
        let sid = json!({"sessionId": "s"});
        assert_eq!(route("session/start", &json!({})), Route::New);
        assert_eq!(route("session/continue", &json!({})), Route::New);
        assert_eq!(route("session/resume", &sid), Route::Resume("s".into()));
        assert_eq!(route("session/delete", &sid), Route::Delete("s".into()));
        assert_eq!(route("turn/start", &sid), Route::Session("s".into()));
        assert_eq!(route("session/pin", &sid), Route::Session("s".into()));
        assert_eq!(
            route("session/fork", &json!({"sourceSessionId": "s"})),
            Route::Session("s".into())
        );
        assert_eq!(route("session/list", &json!({"limit": 5})), Route::Lobby);
        assert_eq!(route("config/read", &json!({})), Route::Lobby);
    }

    #[test]
    fn calls_that_need_a_channel_name_their_session() {
        let sid = json!({"sessionId": "s"});
        assert_eq!(needs_channel("turn/start", &sid), Some("s"));
        assert_eq!(needs_channel("callback/result", &sid), Some("s"));
        // Settings > Network reads and writes the proxy settings through a session like the other settings.
        assert_eq!(needs_channel("config/proxy/read", &sid), Some("s"));
        assert_eq!(needs_channel("config/proxy/write", &sid), Some("s"));
        assert_eq!(
            needs_channel("session/fork", &json!({"sourceSessionId": "f"})),
            Some("f")
        );
        assert_eq!(
            needs_channel("session/pin", &sid),
            None,
            "works from the lobby"
        );
        assert_eq!(needs_channel("session/read", &sid), None);
        for method in [
            "config/write",
            "config/reload",
            "config/fields/read",
            "account/read",
            "identity/read",
        ] {
            assert_eq!(needs_channel(method, &sid), Some("s"), "{method}");
        }
        assert_eq!(needs_channel("config/read", &json!({})), None);
        assert_eq!(needs_channel("turn/start", &json!({})), None);
    }

    /// Vibe's skills service requires the attached session for every skills mutation and for `skills/updates`
    /// (`_skills_service.py`: `_begin_mutation` / `require_session`), so each is reopened and follows an imported
    /// conversation like the reads. An offered update sends `skills/setVersion`.
    #[test]
    fn every_skills_call_that_vibe_answers_only_for_the_attached_session_needs_a_channel() {
        let sid = json!({"sessionId": "s"});
        for method in [
            "skills/setVersion",
            "skills/setAlias",
            "skills/convertLocal",
            "skills/updates",
            "skills/setLatest",
            "skills/import",
            "skills/remove",
            "skills/setEnabled",
        ] {
            assert_eq!(needs_channel(method, &sid), Some("s"), "{method}");
        }
    }
}
