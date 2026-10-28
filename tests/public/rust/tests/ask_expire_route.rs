//! The Debug API's test expiry route (`POST /asks/{id}/expire`) exists only in test builds (the `e2e-script`
//! feature): in a normal build an owner's client cannot dismiss a person's prompt with it, not even as a No.
//! PREDICT (written before the first run): today the route answers in every build: a card ask is expired (200).
mod common;

use common::core::{debug_api, real_core};
use serde_json::{Value, json};
use std::time::Duration;

async fn open(client: &common::http::Client) -> Value {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let response = client.get("/asks").await;
            if let Some(ask) = response.json()["open"].as_array().and_then(|a| a.first()) {
                return ask.clone();
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn the_expiry_route_is_not_in_a_normal_build() {
    let (rig, _model) = real_core(
        "asks-expire-route",
        &[],
        None,
        common::real_vibe::approval_script(),
        |_| {},
    )
    .await;
    let (api, client) = debug_api(&rig.core, &rig.dir).await;
    let started = client
        .command(json!({"type":"startSession","agentConfig":{"agent":"ask"}}))
        .await;
    let sid = started["snapshot"]["sessionId"]
        .as_str()
        .unwrap()
        .to_owned();
    client
        .command(json!({"type":"startTurn", "sessionId":sid, "message":[{"type":"text","text":"write hello.txt"}]}))
        .await;
    let ask = open(&client).await;
    let id = ask["id"].as_str().unwrap();
    let reply = client.post(&format!("/asks/{id}/expire"), &json!({})).await;
    assert_eq!(reply.status, 404, "{}", reply.body);
    assert_eq!(
        client.get("/asks").await.json()["open"][0]["id"],
        ask["id"],
        "the prompt is still open"
    );
    api.shutdown().await;
    rig.core.shutdown().await;
}
