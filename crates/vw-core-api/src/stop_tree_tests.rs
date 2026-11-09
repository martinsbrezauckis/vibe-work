//! Stop races over an in-memory Vibe peer. Exercises real routing/gate and stale running snapshots without sleeps.
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[tokio::test]
async fn a_child_no_survives_the_parent_ending_before_interrupt() {
    for (code, message, already_stopped) in [
        ("conflict", "No active turn", true),
        ("conflict", "Another operation is running", false),
        ("stale_turn", "A different turn is active", false),
    ] {
        let core = crate::offline_core::start(&format!("no-race-{code}-{already_stopped}")).await;
        let shared = &core.shared;
        let state = json!({"session":{"id":"parent","harness":"unified","createdAt":1,
            "bumpedAt":1,"status":{"type":"running","activeTurnId":"old"}},"eventId":0,
            "turns":[{"id":"old","status":"in_progress"}],"history":[],
            "childSessions":[{"id":"child","status":{"type":"blocked"}}]});
        let _ = shared.store.load(state, vw_session::LoadKind::New).unwrap();
        let (client, server) = tokio::io::duplex(16384);
        let (read, write) = tokio::io::split(client);
        let conn =
            vw_rpc::ConnectionBuilder::new(vw_rpc::ConnectionConfig::default()).start(read, write);
        shared.channels.bind_test_peer("parent", conn, (0, 1));
        let peer = tokio::spawn(async move {
            let (read, mut write) = tokio::io::split(server);
            let mut lines = BufReader::new(read).lines();
            let interrupt: Value =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            assert_eq!(interrupt["method"], "turn/interrupt");
            assert_eq!(
                interrupt["params"],
                json!({"sessionId":"parent","expectedTurnId":"old"})
            );
            // The cached turn existed at the read, but Vibe has now settled it. The interrupt's reply,
            // rather than a timer, establishes the exact ordering that used to lose the child's No.
            let reply = json!({"jsonrpc":"2.0","id":interrupt["id"],"error":{"code":code,"message":message}});
            write
                .write_all(format!("{reply}\n").as_bytes())
                .await
                .unwrap();
            if already_stopped {
                let stop: Value =
                    serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
                assert_eq!(stop["method"], "session/subagents/interrupt");
                let reply = json!({"jsonrpc":"2.0","id":stop["id"],"result":{"results":[]}});
                write
                    .write_all(format!("{reply}\n").as_bytes())
                    .await
                    .unwrap();
            }
            // Keep the parent channel alive until Stop closes it.
            let _ = lines.next_line().await;
        });
        let delivered = std::sync::atomic::AtomicBool::new(false);
        let result = crate::ask::stop_with_no(shared, "child", async {
            delivered.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        })
        .await;
        assert_eq!(
            result.is_ok(),
            already_stopped,
            "{code}: {message}: {result:?}"
        );
        assert_eq!(
            delivered.load(std::sync::atomic::Ordering::SeqCst),
            already_stopped
        );
        core.shutdown().await;
        peer.abort();
    }
}
