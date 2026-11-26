//! Close contract against an in-memory multiplex peer. The peer deliberately keeps turns alive on close,
//! matching Vibe 3.1.5; requests and acknowledgements use the production Connection and Multiplexer.
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream};

use crate::{Connection, ConnectionConfig, Multiplexer, MultiplexerBuilder};

pub(crate) struct Peer {
    pub(crate) input: tokio::io::Lines<BufReader<tokio::io::ReadHalf<DuplexStream>>>,
    output: tokio::io::WriteHalf<DuplexStream>,
}

impl Peer {
    pub(crate) async fn next(&mut self) -> Value {
        let line = tokio::time::timeout(Duration::from_secs(2), self.input.next_line())
            .await
            .expect("peer frame in time")
            .unwrap()
            .expect("peer frame before EOF");
        serde_json::from_str(&line).unwrap()
    }

    pub(crate) async fn reply(&mut self, frame: &Value, result: Value) {
        let reply = json!({"channelId":frame["channelId"], "message": {
            "jsonrpc":"2.0", "id":frame["message"]["id"], "result":result
        }});
        self.send(reply).await;
    }

    pub(crate) async fn send(&mut self, frame: Value) {
        self.output
            .write_all(format!("{frame}\n").as_bytes())
            .await
            .unwrap();
    }
}

pub(crate) fn pair() -> (Multiplexer, Peer) {
    let (client, server) = tokio::io::duplex(65536);
    let (output, input) = tokio::io::split(client);
    let mux = MultiplexerBuilder::new(ConnectionConfig {
        interrupt_before_close: true,
        close_timeout: Duration::from_millis(500),
        exit_grace: Duration::ZERO,
        ..ConnectionConfig::default()
    })
    .start(output, input);
    let (input, output) = tokio::io::split(server);
    (
        mux,
        Peer {
            input: BufReader::new(input).lines(),
            output,
        },
    )
}

pub(crate) async fn attached(mux: &Multiplexer, peer: &mut Peer, id: &str) -> Connection {
    let conn = mux.open_channel().unwrap();
    let call = tokio::spawn({
        let conn = conn.clone();
        async move { conn.request("session/start", json!({})).await }
    });
    let frame = peer.next().await;
    peer.reply(
        &frame,
        json!({"state":{"session":{"id":id,"status":{"type":"idle"}}}}),
    )
    .await;
    call.await.unwrap().unwrap();
    conn
}

async fn interrupt_then_idle(peer: &mut Peer, session: &str) {
    let read = peer.next().await;
    assert_eq!(
        read["message"]["method"], "session/read",
        "close must inspect the live turn before sending null"
    );
    assert_eq!(read["message"]["params"]["sessionId"], session);
    peer.reply(&read, json!({"state":{"session":{"id":session,"status":{"type":"running","activeTurnId":"turn-1"}}}})).await;
    let interrupt = peer.next().await;
    assert_eq!(interrupt["message"]["method"], "turn/interrupt");
    assert_eq!(
        interrupt["message"]["params"],
        json!({"sessionId":session,"expectedTurnId":"turn-1"})
    );
    // A second chat must remain usable while this acknowledgement is pending.
    assert!(
        tokio::time::timeout(Duration::from_millis(30), peer.input.next_line())
            .await
            .is_err(),
        "close sent before interrupt acknowledgement"
    );
    peer.reply(&interrupt, json!({"accepted":true})).await;
    let read = peer.next().await;
    assert_eq!(read["message"]["method"], "session/read");
    peer.reply(
        &read,
        json!({"state":{"session":{"id":session,"status":{"type":"idle"}}}}),
    )
    .await;
}

#[tokio::test]
async fn channel_close_interrupts_and_waits_before_null() {
    let (mux, mut peer) = pair();
    let conn = attached(&mux, &mut peer, "chat-a").await;
    let closing = tokio::spawn(async move { conn.close_input().await });
    interrupt_then_idle(&mut peer, "chat-a").await;
    assert_eq!(peer.next().await["message"], Value::Null);
    closing.await.unwrap();
}

#[tokio::test]
async fn last_handle_drop_obeys_the_same_interrupt_contract() {
    let (mux, mut peer) = pair();
    let conn = attached(&mux, &mut peer, "chat-a").await;
    drop(conn);
    interrupt_then_idle(&mut peer, "chat-a").await;
    let close = peer.next().await;
    assert_eq!(close["message"], Value::Null);
    peer.send(close).await;
    assert!(mux.death().is_none());
}

#[tokio::test]
async fn cancelled_close_caller_does_not_cancel_interrupt_cleanup() {
    let (mux, mut peer) = pair();
    let conn = attached(&mux, &mut peer, "chat-a").await;
    let closing = tokio::spawn(async move { conn.close_input().await });
    let read = peer.next().await;
    closing.abort();
    let _ = closing.await;
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
    let close = peer.next().await;
    assert_eq!(close["message"], Value::Null);
    peer.send(close).await;
    assert!(mux.death().is_none());
}

#[tokio::test]
async fn another_chat_stays_usable_and_closing_chat_refuses_new_work() {
    let (mux, mut peer) = pair();
    let one = attached(&mux, &mut peer, "chat-a").await;
    let two = attached(&mux, &mut peer, "chat-b").await;
    let closing = tokio::spawn({
        let one = one.clone();
        async move { one.close_input().await }
    });
    let read = peer.next().await;
    assert!(
        one.request("turn/start", json!({"sessionId":"chat-a"}))
            .await
            .is_err()
    );
    peer.reply(&read, json!({"state":{"session":{"id":"chat-a","status":{"type":"running","activeTurnId":"turn-1"}}}})).await;
    let interrupt = peer.next().await;
    let other = tokio::spawn({
        let two = two.clone();
        async move {
            two.request("runtime/read", json!({"sessionId":"chat-b"}))
                .await
        }
    });
    let request = peer.next().await;
    assert_eq!(request["channelId"], "2");
    peer.reply(&request, json!({"ok":true})).await;
    assert_eq!(other.await.unwrap().unwrap(), json!({"ok":true}));
    peer.reply(&interrupt, json!({"accepted":true})).await;
    let read = peer.next().await;
    peer.reply(
        &read,
        json!({"state":{"session":{"id":"chat-a","status":{"type":"idle"}}}}),
    )
    .await;
    let close = peer.next().await;
    assert_eq!(close["channelId"], "1");
    assert_eq!(close["message"], Value::Null);
    peer.send(close).await;
    closing.await.unwrap();
    assert!(mux.death().is_none());
    assert!(two.death().is_none());
}

#[tokio::test]
async fn process_eof_waits_for_every_channels_interrupt_and_close_ack() {
    let (mux, mut peer) = pair();
    let _one = attached(&mux, &mut peer, "chat-a").await;
    let _two = attached(&mux, &mut peer, "chat-b").await;
    let closing = tokio::spawn({
        let mux = mux.clone();
        async move { mux.close_input().await }
    });
    let mut interrupted = std::collections::BTreeSet::new();
    let mut closed = std::collections::BTreeSet::new();
    while closed.len() < 2 {
        let frame = peer.next().await;
        let channel = frame["channelId"].as_str().unwrap().to_owned();
        let session = if channel == "1" { "chat-a" } else { "chat-b" };
        match frame["message"]["method"].as_str() {
            Some("session/read") => {
                let status = if interrupted.contains(&channel) {
                    json!({"type":"idle"})
                } else {
                    json!({"type":"running","activeTurnId":"turn-1"})
                };
                peer.reply(
                    &frame,
                    json!({"state":{"session":{"id":session,"status":status}}}),
                )
                .await;
            }
            Some("turn/interrupt") => {
                interrupted.insert(channel);
                peer.reply(&frame, json!({"accepted":true})).await;
            }
            _ => {
                assert_eq!(frame["message"], Value::Null);
                assert!(interrupted.contains(&channel));
                closed.insert(channel);
                peer.send(frame).await;
            }
        }
    }
    closing.await.unwrap();
    let eof = tokio::time::timeout(Duration::from_secs(1), peer.input.next_line())
        .await
        .unwrap()
        .unwrap();
    assert!(
        eof.is_none(),
        "process stdin closes only after both channel acknowledgements"
    );
}
