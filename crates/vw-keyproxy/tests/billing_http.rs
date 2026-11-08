//! Loopback billing fixtures verify authentication, refusal boundaries and unchanged upstream responses.
//! The ignored probe runs pinned Vibe's real HTTP gateway and account projection; no live credentials.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use axum::{
    body::Body,
    extract::{Request, State},
    response::Response,
};
use serde_json::Value;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU16, Ordering},
};
use std::time::Duration;
use vw_keyproxy::{KeyCell, KeyNeed, KeyState, ProxyConfig, ProxyHandle, TokenSlot, Upstreams};
use zeroize::Zeroizing;

const KEY: &str = "synthetic-upstream-key-for-billing-tests";
type Seen = Arc<Mutex<Vec<(String, String, String)>>>;

struct Rig {
    proxy: ProxyHandle,
    token: String,
    seen: Seen,
    #[cfg(feature = "plan-usage")]
    status: Arc<AtomicU16>,
    upstream: tokio::task::JoinHandle<()>,
}

async fn upstream(State((seen, status)): State<(Seen, Arc<AtomicU16>)>, req: Request) -> Response {
    let path = req.uri().path().to_owned();
    seen.lock().unwrap().push((
        req.method().to_string(),
        path.clone(),
        req.headers()
            .get("authorization")
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned(),
    ));
    assert!(req.headers().get("cookie").is_none());
    assert!(req.headers().get("x-api-key").is_none());
    let code = status.load(Ordering::SeqCst);
    let fixture: Value = serde_json::from_str(include_str!("support/billing.json")).unwrap();
    let body = if code == 200 {
        fixture[if path.ends_with("subscription") {
            "subscription"
        } else {
            "budget"
        }]
        .to_string()
    } else if code == 201 {
        "invalid billing JSON".to_owned()
    } else {
        format!("fixture {code}")
    };
    Response::builder()
        .status(code)
        .header("content-type", "application/json")
        .header("retry-after", "7")
        .header("location", "/v1/billing/invoices")
        .body(Body::from(body))
        .unwrap()
}

async fn rig(missing: bool) -> Rig {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let status = Arc::new(AtomicU16::new(200));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let app = axum::Router::new()
        .fallback(upstream)
        .with_state((seen.clone(), status.clone()));
    let upstream = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let tokens = Arc::new(TokenSlot::default());
    let token = tokens.rotate().unwrap().to_string();
    let proxy = ProxyHandle::start(
        ProxyConfig {
            upstreams: Upstreams {
                api: base.clone(),
                console: base.clone(),
                chat: base,
            },
            key_wait: Duration::from_secs(1),
            ..ProxyConfig::default()
        },
        tokens,
        KeyCell::new(if missing {
            KeyState::Missing(KeyNeed::Unlock)
        } else {
            KeyState::Ready(Arc::new(Zeroizing::new(KEY.to_owned())))
        }),
    )
    .await
    .unwrap();
    Rig {
        proxy,
        token,
        seen,
        #[cfg(feature = "plan-usage")]
        status,
        upstream,
    }
}

#[tokio::test]
async fn only_the_two_reads_reach_upstream_with_the_current_token() {
    let rig = rig(false).await;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let base = rig.proxy.base_url();
    for path in ["/v1/billing/subscription", "/v1/billing/budget"] {
        for token in [None, Some("wrong"), Some(KEY)] {
            let mut req = client.get(format!("{base}{path}"));
            if let Some(token) = token {
                req = req.bearer_auth(token);
            }
            assert_eq!(req.send().await.unwrap().status(), 401);
        }
        let response = client
            .get(format!("{base}{path}"))
            .bearer_auth(&rig.token)
            .header("cookie", "discard")
            .header("x-api-key", "discard")
            .send()
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            if cfg!(feature = "plan-usage") {
                200
            } else {
                403
            }
        );
        for method in ["POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS", "TRACE"] {
            assert_eq!(
                client
                    .request(method.parse().unwrap(), format!("{base}{path}"))
                    .bearer_auth(&rig.token)
                    .send()
                    .await
                    .unwrap()
                    .status(),
                403,
                "{method} {path}"
            );
        }
    }
    for path in [
        "/v1/billing",
        "/v1/billing/invoices",
        "/v1/billing/budget/",
        "/v1/billing/budget/extra",
        "/v1/billing/%62udget",
        "/v1/billing//budget",
        "/console/v1/billing/budget",
        "/chat/v1/billing/budget",
        "/v1/files",
    ] {
        assert_eq!(
            client
                .get(format!("{base}{path}"))
                .bearer_auth(&rig.token)
                .send()
                .await
                .unwrap()
                .status(),
            403,
            "{path}"
        );
    }
    let seen = rig.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), if cfg!(feature = "plan-usage") { 2 } else { 0 });
    for (method, path, auth) in seen {
        assert_eq!(method, "GET");
        assert!(["/v1/billing/subscription", "/v1/billing/budget"].contains(&path.as_str()));
        assert_eq!(auth, format!("Bearer {KEY}"));
    }
    rig.proxy.tokens().rotate().unwrap();
    assert_eq!(
        client
            .get(format!("{base}/v1/billing/budget"))
            .bearer_auth(&rig.token)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    rig.proxy.shutdown().await;
    rig.upstream.abort();
}

#[cfg(feature = "plan-usage")]
#[tokio::test]
async fn billing_errors_and_redirects_pass_through_without_being_followed() {
    let rig = rig(false).await;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    for code in [302, 401, 403, 404, 429, 500, 503] {
        rig.status.store(code, Ordering::SeqCst);
        let response = client
            .get(format!("{}/v1/billing/budget", rig.proxy.base_url()))
            .bearer_auth(&rig.token)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), code);
        assert_eq!(response.headers()["retry-after"], "7");
        assert_eq!(response.text().await.unwrap(), format!("fixture {code}"));
    }
    assert_eq!(rig.seen.lock().unwrap().len(), 7);
    rig.proxy.shutdown().await;
    rig.upstream.abort();
}

#[cfg(feature = "plan-usage")]
#[tokio::test]
async fn billing_discovery_never_waits_for_a_locked_key() {
    let rig = rig(true).await;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(200))
        .build()
        .unwrap();
    for path in ["/v1/billing/subscription", "/v1/billing/budget"] {
        let response = client
            .get(format!("{}{path}", rig.proxy.base_url()))
            .bearer_auth(&rig.token)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 503);
        assert!(response.text().await.unwrap().contains("Unlock it"));
    }
    assert_eq!(rig.proxy.stats().waited.load(Ordering::SeqCst), 0);
    assert!(rig.seen.lock().unwrap().is_empty());
    rig.proxy.shutdown().await;
    rig.upstream.abort();
}

#[cfg(feature = "plan-usage")]
#[tokio::test]
#[ignore = "needs installed Vibe 3.1.0: source the lane environment and run with --ignored"]
async fn installed_vibe_reads_billing_through_the_proxy() {
    let python = std::env::var_os("VIBE_PYTHON").expect("set VIBE_PYTHON to the pinned runtime");
    let root = std::env::var_os("VW_TEST_ROOT")
        .map(std::path::PathBuf::from)
        .expect("set an isolated VW_TEST_ROOT");
    let home = root.join(format!("billing-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    let rig = rig(false).await;
    for (code, mode) in [
        (200, "both"),
        (200, "subscription-only"),
        (201, "both"),
        (403, "both"),
        (404, "both"),
        (429, "both"),
        (500, "both"),
        (302, "both"),
    ] {
        rig.status.store(code, Ordering::SeqCst);
        let output = tokio::process::Command::new(&python)
            .args(["-I", "-c", include_str!("support/billing_probe.py")])
            .arg(format!("{}/v1", rig.proxy.base_url()))
            .arg(&rig.token)
            .arg(mode)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &home)
            .env("VIBE_HOME", &home)
            .kill_on_drop(true)
            .output();
        let output = tokio::time::timeout(Duration::from_secs(30), output)
            .await
            .expect("Vibe billing probe timed out")
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        println!("Vibe 3.1.0 billing fixture {code} {mode}: {value}");
        if code == 200 {
            assert_eq!(value["subscription"]["vibe_payg_enabled"], true);
            if mode == "both" {
                let fixture: Value =
                    serde_json::from_str(include_str!("support/billing.json")).unwrap();
                assert_eq!(value["budget"], fixture["account"]["account"]["budget"]);
            } else {
                assert_eq!(value["budget"], Value::Null);
            }
        } else {
            assert_eq!(
                value,
                serde_json::json!({"subscription": null, "budget": null})
            );
        }
    }
    let seen = rig.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 15);
    for (method, path, auth) in seen {
        assert_eq!(method, "GET");
        assert!(["/v1/billing/subscription", "/v1/billing/budget"].contains(&path.as_str()));
        assert_eq!(auth, format!("Bearer {KEY}"));
    }
    rig.proxy.shutdown().await;
    rig.upstream.abort();
    std::fs::remove_dir_all(home).unwrap();
}
