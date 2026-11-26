//! A parent channel can still own work after its own turn becomes idle: subagents and queued turns.
//! The peer models the 3.1.5 close behavior; these tests use production multiplex transport.

use std::time::Duration;

use serde_json::{Value, json};

use super::close_tests::{Peer, attached, pair};
use crate::Multiplexer;

async fn read_parent(peer: &mut Peer, state: Value) {
    let read = peer.next().await;
    assert_eq!(read["message"]["method"], "session/read");
    assert_eq!(read["message"]["params"]["sessionId"], "parent");
    peer.reply(&read, json!({"state":state})).await;
}

async fn died_without_null(mux: &Multiplexer, peer: &mut Peer) {
    let death = tokio::time::timeout(Duration::from_secs(2), mux.closed())
        .await
        .expect("uncertain family work marks the process dead");
    assert_eq!(death.cause.kind(), "wedged");
    assert!(
        tokio::time::timeout(Duration::from_millis(40), peer.next())
            .await
            .is_err(),
        "work may continue after a raw channel close"
    );
}

#[tokio::test]
async fn idle_parent_interrupts_running_child_before_channel_close() {
    let (mux, mut peer) = pair();
    let conn = attached(&mux, &mut peer, "parent").await;
    let close = tokio::spawn(async move { conn.close_input().await });
    read_parent(
        &mut peer,
        json!({"session":{"id":"parent","status":{"type":"idle"}},
               "childSessions":[{"id":"child","status":{"type":"running"}}]}),
    )
    .await;
    let interrupt = peer.next().await;
    assert_eq!(
        interrupt["message"]["method"],
        "session/subagents/interrupt"
    );
    assert_eq!(interrupt["message"]["params"]["sessionId"], "parent");
    peer.reply(
        &interrupt,
        json!({"results":[{"sessionId":"child","name":"worker","outcome":"interrupted"}]}),
    )
    .await;
    read_parent(
        &mut peer,
        json!({"session":{"id":"parent","status":{"type":"idle"}},
               "childSessions":[{"id":"child","status":{"type":"idle"}}]}),
    )
    .await;
    assert_eq!(peer.next().await["message"], Value::Null);
    close.await.unwrap();
    assert!(mux.death().is_none());
}

#[tokio::test]
async fn failed_child_interrupt_kills_process_before_channel_close() {
    let (mux, mut peer) = pair();
    let conn = attached(&mux, &mut peer, "parent").await;
    let close = tokio::spawn(async move { conn.close_input().await });
    read_parent(
        &mut peer,
        json!({"session":{"id":"parent","status":{"type":"idle"}},
               "childSessions":[{"id":"child","status":{"type":"blocked"}}]}),
    )
    .await;
    let interrupt = peer.next().await;
    assert_eq!(
        interrupt["message"]["method"],
        "session/subagents/interrupt"
    );
    peer.reply(
        &interrupt,
        json!({"results":[{"sessionId":"child","name":"worker","outcome":"failed"}]}),
    )
    .await;
    died_without_null(&mux, &mut peer).await;
    close.await.unwrap();
}

#[tokio::test]
async fn idle_parent_with_queued_turn_never_closes_live_channel() {
    let (mux, mut peer) = pair();
    let conn = attached(&mux, &mut peer, "parent").await;
    let close = tokio::spawn(async move { conn.close_input().await });
    read_parent(
        &mut peer,
        json!({"session":{"id":"parent","status":{"type":"idle"}},
               "turnQueue":{"items":[{"id":"queued"}]}}),
    )
    .await;
    died_without_null(&mux, &mut peer).await;
    close.await.unwrap();
}
