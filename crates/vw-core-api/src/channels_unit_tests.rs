//! Regression tests for `channels.rs`; uses the parent module and is called by Cargo tests.
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use vw_rpc::{ConnectionBuilder, ConnectionConfig};
use vw_supervisor::{NoSecret, Supervisor, SupervisorConfig};

use super::Channels;

#[tokio::test]
async fn late_failure_cannot_close_replacement_channel() {
    let path = std::env::current_exe().unwrap();
    let config = SupervisorConfig::new(path.clone(), path.clone(), path.clone());
    let supervisor = Arc::new(Supervisor::start_pending(
        config,
        Arc::new(NoSecret),
        "unit test".to_owned(),
    ));
    let (feeds, _received) = mpsc::unbounded_channel();
    let leases = Arc::new(crate::session_lease::Leases::new(&path, None).unwrap());
    let approvals = Arc::new(crate::plugin_approvals::PluginApprovals::new(&path).unwrap());
    let channels = Channels::new(
        supervisor,
        Duration::from_millis(10),
        feeds,
        None,
        leases,
        approvals,
        Arc::new(crate::history_loading::HistoryLoading::new(
            vw_session::SessionStore::new(),
        )),
    );
    let (stdout, _stdout_peer) = tokio::io::duplex(1024);
    let (stdin, _stdin_peer) = tokio::io::duplex(1024);
    let conn = ConnectionBuilder::new(ConnectionConfig::default()).start(stdout, stdin);
    channels.bind("session", conn, (2, 9));

    assert!(!channels.close_if("session", (1, 8)).await);
    assert_eq!(channels.key_of("session"), Some((2, 9)));
}

/// A conversation Vibe imported under a new id: the attached requests the window sends with the id it knows go
/// to the new id, an offered skill update included.
#[tokio::test]
async fn an_offered_skill_update_follows_an_imported_conversation() {
    let path = std::env::current_exe().unwrap();
    let config = SupervisorConfig::new(path.clone(), path.clone(), path.clone());
    let supervisor = Arc::new(Supervisor::start_pending(
        config,
        Arc::new(NoSecret),
        "unit test".to_owned(),
    ));
    let (feeds, _received) = mpsc::unbounded_channel();
    let leases = Arc::new(crate::session_lease::Leases::new(&path, None).unwrap());
    let approvals = Arc::new(crate::plugin_approvals::PluginApprovals::new(&path).unwrap());
    let channels = Channels::new(
        supervisor,
        Duration::from_millis(10),
        feeds,
        None,
        leases,
        approvals,
        Arc::new(crate::history_loading::HistoryLoading::new(
            vw_session::SessionStore::new(),
        )),
    );
    super::locked(&channels.inner)
        .imported
        .insert("cli-id".into(), "imported-id".into());
    for method in ["skills/setVersion", "skills/setLatest", "skills/installed"] {
        let mut params = serde_json::json!({"sessionId": "cli-id", "name": "release-notes"});
        channels.canonical_params(method, &mut params);
        assert_eq!(params["sessionId"], "imported-id", "{method}");
    }
    // A composer's runtime read for the chosen row follows the conversation to its new id too: under the old
    // id it would go to the lobby, which refuses it. Stored-history reads keep their own id.
    let mut params = serde_json::json!({"sessionId": "cli-id"});
    channels.canonical_params("runtime/read", &mut params);
    assert_eq!(params["sessionId"], "imported-id", "runtime/read");
    let mut params = serde_json::json!({"sessionId": "cli-id"});
    channels.canonical_params("session/read", &mut params);
    assert_eq!(params["sessionId"], "cli-id", "session/read");
}
