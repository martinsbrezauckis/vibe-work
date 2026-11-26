//! Adverse close receipts: a failed interrupt or uncertain channel ends the process before null.
//! Uses the same in-memory multiplex peer as `close_tests`, with no real Vibe process.

use std::time::Duration;

use serde_json::{Value, json};

use super::close_tests::{Peer, attached, pair};
use crate::{Multiplexer, RequestOptions, TimeoutPolicy};

async fn death_without_close(mux: &Multiplexer, peer: &mut Peer) {
    let death = tokio::time::timeout(Duration::from_secs(2), mux.closed())
        .await
        .expect("failed close marks the process dead promptly");
    assert_eq!(death.cause.kind(), "wedged");
    assert!(
        tokio::time::timeout(Duration::from_millis(40), peer.next())
            .await
            .is_err(),
        "a failed close must not send a channel-close or stdin EOF frame"
    );
}

async fn first_running_read(peer: &mut Peer, session: &str, turn: &str) {
    let read = peer.next().await;
    assert_eq!(read["message"]["method"], "session/read");
    assert_eq!(read["message"]["params"]["sessionId"], session);
    peer.reply(
        &read,
        json!({"state":{"session":{"id":session,"status":{"type":"running","activeTurnId":turn}}}}),
    )
    .await;
}

#[tokio::test]
async fn missing_or_empty_active_turn_fails_closed() {
    for status in [
        json!({}),
        json!({"type":"running"}),
        json!({"type":"running","activeTurnId":""}),
    ] {
        let (mux, mut peer) = pair();
        let conn = attached(&mux, &mut peer, "chat-a").await;
        let close = tokio::spawn(async move { conn.close_input().await });
        let read = peer.next().await;
        assert_eq!(read["message"]["method"], "session/read");
        peer.reply(
            &read,
            json!({"state":{"session":{"id":"chat-a","status":status}}}),
        )
        .await;
        death_without_close(&mux, &mut peer).await;
        close.await.unwrap();
    }
}

#[tokio::test]
async fn refused_interrupt_fails_closed_even_if_the_turn_later_looks_idle() {
    let (mux, mut peer) = pair();
    let conn = attached(&mux, &mut peer, "chat-a").await;
    let close = tokio::spawn(async move { conn.close_input().await });
    first_running_read(&mut peer, "chat-a", "turn-1").await;
    let interrupt = peer.next().await;
    assert_eq!(interrupt["message"]["method"], "turn/interrupt");
    peer.reply(&interrupt, json!({"accepted":false})).await;
    death_without_close(&mux, &mut peer).await;
    close.await.unwrap();
}

#[tokio::test]
async fn stale_turn_conflict_fails_closed() {
    let (mux, mut peer) = pair();
    let conn = attached(&mux, &mut peer, "chat-a").await;
    let close = tokio::spawn(async move { conn.close_input().await });
    first_running_read(&mut peer, "chat-a", "old-turn").await;
    let interrupt = peer.next().await;
    assert_eq!(interrupt["message"]["params"]["expectedTurnId"], "old-turn");
    peer.send(json!({
        "channelId":interrupt["channelId"],
        "message":{
            "jsonrpc":"2.0", "id":interrupt["message"]["id"],
            "error":{"code":"conflict","message":"Expected turn does not match"}
        }
    }))
    .await;
    death_without_close(&mux, &mut peer).await;
    close.await.unwrap();
}

#[tokio::test]
async fn naturally_finished_turn_is_verified_idle_after_no_active_turn_conflict() {
    let (mux, mut peer) = pair();
    let conn = attached(&mux, &mut peer, "chat-a").await;
    let close = tokio::spawn(async move { conn.close_input().await });
    first_running_read(&mut peer, "chat-a", "turn-1").await;
    let interrupt = peer.next().await;
    peer.send(json!({"channelId":interrupt["channelId"], "message":{
        "jsonrpc":"2.0", "id":interrupt["message"]["id"],
        "error":{"code":"conflict","message":"No active turn"}
    }}))
    .await;
    let read = peer.next().await;
    assert_eq!(read["message"]["method"], "session/read");
    peer.reply(
        &read,
        json!({"state":{"session":{"id":"chat-a","status":{"type":"idle"}}}}),
    )
    .await;
    let frame = peer.next().await;
    assert_eq!(frame["message"], Value::Null);
    peer.send(frame).await;
    close.await.unwrap();
    assert!(
        mux.death().is_none(),
        "a naturally finished turn must not kill another chat's server"
    );
}

#[tokio::test]
async fn process_shutdown_retains_channel_writer_after_last_real_handle_drops() {
    let (mux, mut peer) = pair();
    let conn = attached(&mux, &mut peer, "chat-a").await;
    let close = tokio::spawn({
        let mux = mux.clone();
        async move { mux.close_input().await }
    });
    let read = peer.next().await;
    drop(conn);
    peer.reply(&read, json!({"state":{"session":{"id":"chat-a","status":{"type":"running","activeTurnId":"turn-1"}}}})).await;
    let interrupt = peer.next().await;
    assert_eq!(interrupt["message"]["method"], "turn/interrupt");
    peer.reply(&interrupt, json!({"accepted":true})).await;
    let read = peer.next().await;
    peer.reply(
        &read,
        json!({"state":{"session":{"id":"chat-a","status":{"type":"idle"}}}}),
    )
    .await;
    let frame = peer.next().await;
    assert_eq!(frame["message"], Value::Null);
    peer.send(frame).await;
    close.await.unwrap();
    assert!(mux.death().is_none());
}

#[tokio::test]
async fn unanswered_interrupt_hits_the_bound_and_fails_closed() {
    let (mux, mut peer) = pair();
    let conn = attached(&mux, &mut peer, "chat-a").await;
    let close = tokio::spawn(async move { conn.close_input().await });
    first_running_read(&mut peer, "chat-a", "turn-1").await;
    assert_eq!(peer.next().await["message"]["method"], "turn/interrupt");
    death_without_close(&mux, &mut peer).await;
    close.await.unwrap();
}

#[tokio::test]
async fn direct_stop_timeout_kills_the_process_even_before_channel_close() {
    for method in ["turn/interrupt", "session/subagents/interrupt"] {
        for options in [
            RequestOptions::default(),
            RequestOptions {
                timeout: Some(Duration::from_secs(30)),
                on_timeout: Some(TimeoutPolicy::Fail),
            },
        ] {
            let (mux, mut peer) = pair();
            let conn = attached(&mux, &mut peer, "chat-a").await;
            let params = if method == "turn/interrupt" {
                json!({"sessionId":"chat-a","expectedTurnId":"turn-1"})
            } else {
                json!({"sessionId":"chat-a","reason":"Stopped by the user"})
            };
            let request =
                tokio::spawn(async move { conn.request_with(method, params, options).await });
            assert_eq!(peer.next().await["message"]["method"], method);
            tokio::time::timeout(Duration::from_secs(1), mux.closed())
                .await
                .expect(
                    "a direct app Stop must kill an unresponsive server within its close budget",
                );
            assert!(request.await.unwrap().is_err());
            assert!(
                tokio::time::timeout(Duration::from_millis(40), peer.next())
                    .await
                    .is_err(),
                "a direct failed Stop must not send a raw channel close"
            );
            println!(
                "DIRECT_STOP_TIMEOUT_PASS method={method} explicit_override={}",
                options.timeout.is_some()
            );
        }
    }
}

#[tokio::test]
async fn accepted_interrupt_waits_for_idle_without_interrupting_the_same_turn_twice() {
    let (mux, mut peer) = pair();
    let conn = attached(&mux, &mut peer, "chat-a").await;
    let close = tokio::spawn(async move { conn.close_input().await });
    first_running_read(&mut peer, "chat-a", "turn-1").await;
    let interrupt = peer.next().await;
    assert_eq!(interrupt["message"]["method"], "turn/interrupt");
    peer.reply(&interrupt, json!({"accepted":true})).await;
    // Vibe can acknowledge the request before the status event settles.
    let still_running = peer.next().await;
    assert_eq!(still_running["message"]["method"], "session/read");
    peer.reply(
        &still_running,
        json!({"state":{"session":{"id":"chat-a","status":{"type":"running","activeTurnId":"turn-1"}}}}),
    )
    .await;
    let idle_read = peer.next().await;
    assert_eq!(idle_read["message"]["method"], "session/read");
    peer.reply(
        &idle_read,
        json!({"state":{"session":{"id":"chat-a","status":{"type":"idle"}}}}),
    )
    .await;
    assert_eq!(peer.next().await["message"], Value::Null);
    close.await.unwrap();
    assert!(mux.death().is_none());
}

#[tokio::test]
async fn cancelled_attaching_request_makes_channel_uncertain() {
    let (mux, mut peer) = pair();
    let conn = mux.open_channel().unwrap();
    let request = tokio::spawn({
        let conn = conn.clone();
        async move { conn.request("session/start", json!({})).await }
    });
    assert_eq!(peer.next().await["message"]["method"], "session/start");
    request.abort();
    let _ = request.await;
    let close = tokio::spawn(async move { conn.close_input().await });
    death_without_close(&mux, &mut peer).await;
    close.await.unwrap();
}

#[tokio::test]
async fn cancelled_state_reads_do_not_make_a_known_attachment_uncertain() {
    for method in ["session/read", "runtime/read"] {
        let (mux, mut peer) = pair();
        let conn = attached(&mux, &mut peer, "chat-a").await;
        let read = tokio::spawn({
            let conn = conn.clone();
            async move { conn.request(method, json!({"sessionId":"chat-a"})).await }
        });
        assert_eq!(peer.next().await["message"]["method"], method);
        read.abort();
        let _ = read.await;
        let close = tokio::spawn(async move { conn.close_input().await });
        first_running_read(&mut peer, "chat-a", "turn-1").await;
        let interrupt = peer.next().await;
        peer.reply(&interrupt, json!({"accepted":true})).await;
        let read = peer.next().await;
        peer.reply(
            &read,
            json!({"state":{"session":{"id":"chat-a","status":{"type":"idle"}}}}),
        )
        .await;
        let frame = peer.next().await;
        assert_eq!(frame["message"], Value::Null);
        peer.send(frame).await;
        close.await.unwrap();
        assert!(mux.death().is_none());
    }
}

#[tokio::test]
async fn failed_resume_with_unknown_attachment_kills_before_any_null() {
    for code in ["invalid_params", "internal_error"] {
        let (mux, mut peer) = pair();
        let conn = mux.open_channel().unwrap();
        let resume = tokio::spawn({
            let conn = conn.clone();
            async move {
                conn.request("session/resume", json!({"sessionId":"recovering"}))
                    .await
            }
        });
        let frame = peer.next().await;
        peer.send(json!({"channelId":frame["channelId"], "message":{
            "jsonrpc":"2.0", "id":frame["message"]["id"],
            "error":{"code":code,"message":"resume failed without an attachment receipt"}
        }}))
        .await;
        assert!(resume.await.unwrap().is_err());
        let close = tokio::spawn(async move { conn.close_input().await });
        death_without_close(&mux, &mut peer).await;
        close.await.unwrap();
    }
}

#[tokio::test]
async fn failed_and_archived_parents_close_without_killing_another_chat() {
    for status in ["failed", "archived"] {
        let (mux, mut peer) = pair();
        let conn = attached(&mux, &mut peer, "ended").await;
        let other = attached(&mux, &mut peer, "other").await;
        let close = tokio::spawn(async move { conn.close_input().await });
        let read = peer.next().await;
        peer.reply(
            &read,
            json!({"state":{"session":{"id":"ended","status":{"type":status}}}}),
        )
        .await;
        let frame = peer.next().await;
        assert_eq!(frame["message"], Value::Null);
        peer.send(frame).await;
        close.await.unwrap();
        assert!(mux.death().is_none());
        assert!(other.death().is_none());
    }
}

#[tokio::test]
async fn failed_and_archived_children_are_quiet() {
    for status in ["failed", "archived"] {
        let (mux, mut peer) = pair();
        let conn = attached(&mux, &mut peer, "parent").await;
        let close = tokio::spawn(async move { conn.close_input().await });
        let read = peer.next().await;
        peer.reply(
            &read,
            json!({"state":{"session":{"id":"parent","status":{"type":"idle"}},
            "childSessions":[{"id":"child","status":{"type":status}}]}}),
        )
        .await;
        let frame = peer.next().await;
        assert_eq!(frame["message"], Value::Null);
        peer.send(frame).await;
        close.await.unwrap();
        assert!(mux.death().is_none());
    }
}

#[tokio::test]
async fn wedged_callback_kills_process_without_raw_channel_close() {
    let (mux, mut peer) = pair();
    let conn = attached(&mux, &mut peer, "chat-a").await;
    let request = tokio::spawn(async move {
        conn.request_with(
            "callback/result",
            json!({"sessionId":"chat-a","result":{}}),
            RequestOptions {
                timeout: Some(Duration::from_millis(30)),
                on_timeout: Some(TimeoutPolicy::ServerDead),
            },
        )
        .await
    });
    assert_eq!(peer.next().await["message"]["method"], "callback/result");
    assert!(request.await.unwrap().is_err());
    death_without_close(&mux, &mut peer).await;
}

#[tokio::test]
async fn nonattaching_fork_keeps_the_source_binding() {
    let (mux, mut peer) = pair();
    let conn = attached(&mux, &mut peer, "source").await;
    let fork = tokio::spawn({
        let conn = conn.clone();
        async move {
            conn.request(
                "session/fork",
                json!({"sourceSessionId":"source","attach":false}),
            )
            .await
        }
    });
    let request = peer.next().await;
    assert_eq!(request["message"]["method"], "session/fork");
    peer.reply(
        &request,
        json!({"state":{"session":{"id":"forked","status":{"type":"idle"}}}}),
    )
    .await;
    fork.await.unwrap().unwrap();
    let close = tokio::spawn(async move { conn.close_input().await });
    let read = peer.next().await;
    assert_eq!(read["message"]["method"], "session/read");
    assert_eq!(read["message"]["params"]["sessionId"], "source");
    peer.reply(
        &read,
        json!({"state":{"session":{"id":"source","status":{"type":"idle"}}}}),
    )
    .await;
    assert_eq!(peer.next().await["message"], Value::Null);
    close.await.unwrap();
}

#[tokio::test]
async fn close_reads_idle_again_after_the_last_admitted_request_answers() {
    let (mux, mut peer) = pair();
    let conn = attached(&mux, &mut peer, "source").await;
    let request = tokio::spawn({
        let conn = conn.clone();
        async move {
            conn.request("config/reload", json!({"sessionId":"source"}))
                .await
        }
    });
    let reload = peer.next().await;
    assert_eq!(reload["message"]["method"], "config/reload");
    let close = tokio::spawn(async move { conn.close_input().await });
    let initial = peer.next().await;
    assert_eq!(initial["message"]["method"], "session/read");
    peer.reply(
        &initial,
        json!({"state":{"session":{"id":"source","status":{"type":"idle"}}}}),
    )
    .await;
    let read = peer.next().await;
    assert_eq!(
        read["message"]["method"], "session/read",
        "an idle read while reload is pending must not close the channel"
    );
    peer.reply(&reload, json!({})).await;
    request.await.unwrap().unwrap();
    peer.reply(
        &read,
        json!({"state":{"session":{"id":"source","status":{"type":"idle"}}}}),
    )
    .await;
    // The read already in flight started before reload answered, so one final read is required.
    let final_read = peer.next().await;
    assert_eq!(final_read["message"]["method"], "session/read");
    peer.reply(
        &final_read,
        json!({"state":{"session":{"id":"source","status":{"type":"idle"}}}}),
    )
    .await;
    assert_eq!(peer.next().await["message"], Value::Null);
    close.await.unwrap();
}

#[tokio::test]
async fn close_interrupts_a_running_turn_while_compaction_is_still_pending() {
    let (mux, mut peer) = pair();
    let conn = attached(&mux, &mut peer, "source").await;
    let compact = tokio::spawn({
        let conn = conn.clone();
        async move {
            conn.request("session/compact", json!({"sessionId":"source"}))
                .await
        }
    });
    let compact_frame = peer.next().await;
    assert_eq!(compact_frame["message"]["method"], "session/compact");
    let close = tokio::spawn(async move { conn.close_input().await });
    first_running_read(&mut peer, "source", "turn-1").await;
    let interrupt = peer.next().await;
    assert_eq!(interrupt["message"]["method"], "turn/interrupt");
    peer.reply(&interrupt, json!({"accepted":true})).await;
    peer.reply(&compact_frame, json!({})).await;
    compact.await.unwrap().unwrap();
    let read = peer.next().await;
    assert_eq!(read["message"]["method"], "session/read");
    peer.reply(
        &read,
        json!({"state":{"session":{"id":"source","status":{"type":"idle"}}}}),
    )
    .await;
    assert_eq!(peer.next().await["message"], Value::Null);
    close.await.unwrap();
    assert!(mux.death().is_none());
}
