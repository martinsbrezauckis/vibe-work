//! Live key-wait accounting follows HTTP cancellation, shutdown and key timeout.
//! Uses synthetic proxy tokens and missing keys; no request can reach an upstream.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;
use vw_keyproxy::{KeyCell, KeyNeed, KeyState, ProxyConfig, ProxyHandle, TokenSlot};

async fn waiting(proxy: &ProxyHandle, expected: u64) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while proxy.stats().waiting.load(Ordering::Relaxed) != expected {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("live key waits did not reach the expected count");
}

#[tokio::test]
async fn simultaneous_waits_drain_on_client_cancel_and_shutdown() {
    for need in [KeyNeed::Setup, KeyNeed::Unlock] {
        let tokens = Arc::new(TokenSlot::default());
        let token = tokens.rotate().unwrap();
        let proxy = ProxyHandle::start(
            ProxyConfig::default(),
            tokens,
            KeyCell::new(KeyState::Missing(need)),
        )
        .await
        .unwrap();
        let request = || {
            let client = reqwest::Client::new();
            let url = format!("{}/v1/chat/completions", proxy.base_url());
            let token = token.clone();
            tokio::spawn(async move {
                client
                    .post(url)
                    .bearer_auth(token.as_str())
                    .header("content-type", "application/json")
                    .body(r#"{"messages":[]}"#)
                    .send()
                    .await
                    .unwrap()
            })
        };
        let cancelled = request();
        let stopped = request();
        waiting(&proxy, 2).await;
        cancelled.abort();
        assert!(cancelled.await.unwrap_err().is_cancelled());
        waiting(&proxy, 1).await;
        proxy.begin_shutdown();
        let response = tokio::time::timeout(Duration::from_secs(2), stopped)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(response.status(), 422);
        assert!(
            response
                .text()
                .await
                .unwrap()
                .contains("vibe_work_proxy_stopping")
        );
        waiting(&proxy, 0).await;
        assert_eq!(proxy.stats().waited.load(Ordering::Relaxed), 2);
        assert_eq!(proxy.stats().stopping.load(Ordering::Relaxed), 1);
        assert_eq!(proxy.stats().forwarded.load(Ordering::Relaxed), 0);
        proxy.shutdown().await;
    }
}

#[tokio::test]
async fn key_timeout_and_nonwaiting_requests_leave_no_live_waits() {
    let tokens = Arc::new(TokenSlot::default());
    let token = tokens.rotate().unwrap();
    let proxy = ProxyHandle::start(
        ProxyConfig {
            key_wait: Duration::from_millis(50),
            ..ProxyConfig::default()
        },
        tokens,
        KeyCell::default(),
    )
    .await
    .unwrap();
    let client = reqwest::Client::new();
    for path in ["/v1/models", "/v1/model-catalog"] {
        let response = client
            .get(format!("{}{path}", proxy.base_url()))
            .bearer_auth(token.as_str())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 503);
        assert_eq!(proxy.stats().waiting.load(Ordering::Relaxed), 0);
    }
    for token in [None, Some("")] {
        let mut request = client.get(format!("{}/v1/models", proxy.base_url()));
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        assert_eq!(request.send().await.unwrap().status(), 401);
    }
    assert_eq!(proxy.stats().waited.load(Ordering::Relaxed), 1);
    assert_eq!(proxy.stats().waiting.load(Ordering::Relaxed), 0);
    assert_eq!(proxy.stats().forwarded.load(Ordering::Relaxed), 0);
    proxy.shutdown().await;
}
