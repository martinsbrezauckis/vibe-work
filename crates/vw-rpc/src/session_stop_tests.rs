//! An acknowledged session/stop ends its channel without ending other chats on the process.
//! Uses the production multiplex transport and the in-memory close peer; errors stay fail-closed.
use std::time::Duration;

use serde_json::{Value, json};

use super::close_tests::{attached, pair};
use crate::{DeathCause, RpcError};

#[tokio::test]
async fn acknowledged_session_stop_keeps_the_process_and_second_chat_alive() {
    let (mux, mut peer) = pair();
    let one = attached(&mux, &mut peer, "chat-a").await;
    let two = attached(&mux, &mut peer, "chat-b").await;
    let stop = tokio::spawn({
        let one = one.clone();
        async move {
            one.request("session/stop", json!({"sessionId":"chat-a"}))
                .await
        }
    });
    let frame = peer.next().await;
    assert_eq!(frame["message"]["method"], "session/stop");
    peer.reply(&frame, json!({})).await;
    peer.send(json!({"channelId":frame["channelId"],"message":null}))
        .await;
    stop.await.unwrap().unwrap();
    let closed = one.closed().await;
    assert_eq!(closed.cause, DeathCause::ClosedByClient);
    assert!(
        mux.death().is_none(),
        "a stopped chat must not restart every chat's server"
    );
    assert!(two.death().is_none());
    let read = tokio::spawn({
        let two = two.clone();
        async move {
            two.request("runtime/read", json!({"sessionId":"chat-b"}))
                .await
        }
    });
    let frame = peer.next().await;
    assert_eq!(frame["channelId"], "2");
    peer.reply(&frame, json!({"ok":true})).await;
    assert_eq!(read.await.unwrap().unwrap(), json!({"ok":true}));
}

#[tokio::test]
async fn acknowledged_session_stop_waits_for_the_channel_close_before_returning() {
    let (mux, mut peer) = pair();
    let one = attached(&mux, &mut peer, "chat-a").await;
    let mut stop = tokio::spawn({
        let one = one.clone();
        async move {
            one.request("session/stop", json!({"sessionId":"chat-a"}))
                .await
        }
    });
    let frame = peer.next().await;
    peer.reply(&frame, json!({})).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(30), &mut stop)
            .await
            .is_err(),
        "a successful stop must wait until its old channel releases the chat"
    );
    let work = tokio::time::timeout(
        Duration::from_millis(100),
        one.request("turn/start", json!({"sessionId":"chat-a"})),
    )
    .await
    .expect("acknowledged stop fences new work immediately");
    assert!(matches!(work, Err(RpcError::ServerDead(_))));
    peer.send(json!({"channelId":frame["channelId"],"message":null}))
        .await;
    stop.await.unwrap().unwrap();
    assert!(mux.death().is_none());
}

#[tokio::test]
async fn successful_stop_without_a_close_receipt_is_bounded_and_fails_closed() {
    let (mux, mut peer) = pair();
    let one = attached(&mux, &mut peer, "chat-a").await;
    let stop = tokio::spawn(async move {
        one.request("session/stop", json!({"sessionId":"chat-a"}))
            .await
    });
    let frame = peer.next().await;
    peer.reply(&frame, json!({})).await;
    let result = tokio::time::timeout(Duration::from_secs(2), stop)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(result, Err(RpcError::ServerDead(_))), "{result:?}");
    assert!(mux.death().is_some());
}

#[tokio::test]
async fn failed_or_unacknowledged_stop_still_ends_the_process() {
    for acknowledged_error in [false, true] {
        let (mux, mut peer) = pair();
        let one = attached(&mux, &mut peer, "chat-a").await;
        let stop = tokio::spawn({
            let one = one.clone();
            async move {
                one.request("session/stop", json!({"sessionId":"chat-a"}))
                    .await
            }
        });
        let frame = peer.next().await;
        if acknowledged_error {
            peer.send(json!({"channelId":frame["channelId"], "message":{
                "jsonrpc":"2.0", "id":frame["message"]["id"],
                "error":{"code":"conflict","message":"stop refused"}
            }}))
            .await;
        }
        peer.send(json!({"channelId":frame["channelId"],"message":null}))
            .await;
        assert!(stop.await.unwrap().is_err());
        one.closed().await;
        assert!(
            mux.death().is_some(),
            "an unverified stop must retain fail-closed teardown"
        );
    }
}

#[tokio::test]
async fn error_close_after_successful_stop_still_ends_the_process() {
    let (mux, mut peer) = pair();
    let one = attached(&mux, &mut peer, "chat-a").await;
    let stop = tokio::spawn({
        let one = one.clone();
        async move {
            one.request("session/stop", json!({"sessionId":"chat-a"}))
                .await
        }
    });
    let frame = peer.next().await;
    peer.reply(&frame, json!({})).await;
    peer.send(json!({"channelId":frame["channelId"],"message":Value::Null,"error":"stop failed"}))
        .await;
    assert!(matches!(stop.await.unwrap(), Err(RpcError::ServerDead(_))));
    assert!(matches!(
        one.closed().await.cause,
        DeathCause::ChannelClosed { error: Some(_) }
    ));
    assert!(mux.death().is_some());
}

#[tokio::test]
async fn missing_empty_or_mismatched_stop_target_does_not_authorize_channel_close() {
    for params in [
        json!({}),
        json!({"sessionId":""}),
        json!({"sessionId":"chat-b"}),
    ] {
        let (mux, mut peer) = pair();
        let one = attached(&mux, &mut peer, "chat-a").await;
        let stop = tokio::spawn({
            let one = one.clone();
            async move { one.request("session/stop", params).await }
        });
        let frame = peer.next().await;
        peer.reply(&frame, json!({})).await;
        peer.send(json!({"channelId":frame["channelId"],"message":null}))
            .await;
        let _ = stop.await.unwrap();
        one.closed().await;
        assert!(
            mux.death().is_some(),
            "a stop without its verified chat target is not a safe receipt"
        );
    }
}

#[tokio::test]
async fn cancelled_stop_caller_still_has_a_bounded_owned_close_watch() {
    let (mux, mut peer) = pair();
    let one = attached(&mux, &mut peer, "chat-a").await;
    let stop = tokio::spawn({
        let one = one.clone();
        async move {
            one.request("session/stop", json!({"sessionId":"chat-a"}))
                .await
        }
    });
    let frame = peer.next().await;
    peer.reply(&frame, json!({})).await;
    tokio::time::timeout(Duration::from_secs(1), async {
        while !one.shared.is_closing() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the stop receipt fences the acknowledged channel");
    stop.abort();
    let _ = stop.await;
    tokio::time::timeout(Duration::from_secs(2), mux.closed())
        .await
        .expect("cancelling a caller must not cancel the close bound");
}

#[tokio::test]
async fn cancelled_stop_caller_with_a_clean_receipt_preserves_the_second_chat() {
    let (mux, mut peer) = pair();
    let one = attached(&mux, &mut peer, "chat-a").await;
    let two = attached(&mux, &mut peer, "chat-b").await;
    let stop = tokio::spawn({
        let one = one.clone();
        async move {
            one.request("session/stop", json!({"sessionId":"chat-a"}))
                .await
        }
    });
    let frame = peer.next().await;
    peer.reply(&frame, json!({})).await;
    tokio::time::timeout(Duration::from_secs(1), async {
        while !one.shared.is_closing() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the acknowledged stop is fenced before cancelling its caller");
    stop.abort();
    let _ = stop.await;
    peer.send(json!({"channelId":frame["channelId"],"message":null}))
        .await;
    one.closed().await;
    assert!(mux.death().is_none());
    assert!(two.death().is_none());
}

#[tokio::test]
async fn pending_or_cancelled_work_keeps_stop_fail_closed() {
    for cancelled in [false, true] {
        let (mux, mut peer) = pair();
        let one = attached(&mux, &mut peer, "chat-a").await;
        let mut work = tokio::spawn({
            let one = one.clone();
            async move {
                one.request("turn/start", json!({"sessionId":"chat-a"}))
                    .await
            }
        });
        assert_eq!(peer.next().await["message"]["method"], "turn/start");
        if cancelled {
            work.abort();
            let _ = (&mut work).await;
        }
        let stop = tokio::spawn({
            let one = one.clone();
            async move {
                one.request("session/stop", json!({"sessionId":"chat-a"}))
                    .await
            }
        });
        let frame = peer.next().await;
        peer.reply(&frame, json!({})).await;
        peer.send(json!({"channelId":frame["channelId"],"message":null}))
            .await;
        let _ = stop.await.unwrap();
        one.closed().await;
        assert!(
            mux.death().is_some(),
            "uncertain or pending work must not borrow a stop receipt"
        );
        if !cancelled {
            let _ = work.await;
        }
    }
}

#[tokio::test]
async fn acknowledged_stop_resolves_a_concurrent_close_read_without_restart() {
    let (mux, mut peer) = pair();
    let one = attached(&mux, &mut peer, "chat-a").await;
    let stop = tokio::spawn({
        let one = one.clone();
        async move {
            one.request("session/stop", json!({"sessionId":"chat-a"}))
                .await
        }
    });
    let frame = peer.next().await;
    let closing = tokio::spawn({
        let one = one.clone();
        async move { one.close_input().await }
    });
    let read = peer.next().await;
    assert_eq!(read["message"]["method"], "session/read");
    peer.reply(&frame, json!({})).await;
    peer.send(json!({"channelId":frame["channelId"],"message":null}))
        .await;
    stop.await.unwrap().unwrap();
    closing.await.unwrap();
    assert!(
        mux.death().is_none(),
        "a verified stop supersedes the close task's pending state read"
    );
}

#[tokio::test]
async fn raw_transport_keeps_its_stop_response_and_channel_close_behavior() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let (client, server) = tokio::io::duplex(65536);
    let (output, input) = tokio::io::split(client);
    let mux =
        crate::MultiplexerBuilder::new(crate::ConnectionConfig::default()).start(output, input);
    let one = mux.open_channel().unwrap();
    let (input, mut output) = tokio::io::split(server);
    let mut lines = BufReader::new(input).lines();
    let stop = tokio::spawn({
        let one = one.clone();
        async move {
            one.request("session/stop", json!({"sessionId":"chat-a"}))
                .await
        }
    });
    let frame: Value = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
    let reply = json!({"channelId":frame["channelId"],"message":{
        "jsonrpc":"2.0","id":frame["message"]["id"],"result":{}
    }});
    output
        .write_all(format!("{reply}\n").as_bytes())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), stop)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let close = json!({"channelId":frame["channelId"],"message":null});
    output
        .write_all(format!("{close}\n").as_bytes())
        .await
        .unwrap();
    assert_eq!(
        one.closed().await.cause,
        DeathCause::ChannelClosed { error: None }
    );
    assert!(mux.death().is_none());
}
