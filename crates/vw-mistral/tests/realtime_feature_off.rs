//! A proxy-aware app build is required before a direct legacy WebSocket may be used.
#![cfg(not(feature = "proxy-transports"))]
#![allow(clippy::unwrap_used)]

use futures_util::{SinkExt as _, StreamExt as _};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;
use vw_mistral::network::{NetworkSettings, set_current};
use vw_mistral::realtime;

#[tokio::test]
async fn legacy_transport_refuses_a_chosen_proxy_without_opening_direct() {
    let destination = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = destination.local_addr().unwrap().port();
    set_current(NetworkSettings::from_lookup(|key| {
        (key == "HTTPS_PROXY").then(|| "http://127.0.0.1:9".to_owned())
    }));
    let result = tokio::time::timeout(
        Duration::from_millis(300),
        realtime::connect(&format!("https://127.0.0.1:{port}"), "fixture-key"),
    )
    .await
    .expect("a selected proxy must be refused promptly");
    let Err(error) = result else {
        panic!("a selected proxy was bypassed");
    };
    assert_eq!(
        error.to_string(),
        "Could not reach Mistral: this build cannot route live transcription through the chosen proxy."
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(100), destination.accept())
            .await
            .is_err()
    );

    set_current(NetworkSettings::default());
    let direct = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = direct.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (tcp, _) = direct.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
        ws.send(Message::Text(r#"{"type":"session.created"}"#.into()))
            .await
            .unwrap();
        let update = ws.next().await.unwrap().unwrap().into_text().unwrap();
        assert!(update.contains("session.update"));
        ws.send(Message::Text(
            r#"{"type":"transcription.text.delta","text":"direct"}"#.into(),
        ))
        .await
        .unwrap();
    });
    let (sender, mut receiver) =
        realtime::connect(&format!("http://127.0.0.1:{port}"), "fixture-key")
            .await
            .unwrap();
    assert_eq!(
        receiver.next().await,
        realtime::RealtimeEvent::Delta("direct".to_owned())
    );
    sender.close().await;
    server.await.unwrap();
}
