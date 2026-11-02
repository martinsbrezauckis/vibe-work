//! A read that lands while the page loads again (a news front page that refreshes itself, a tab the browser
//! reloads) waits for that load, bounded, instead of answering the half-built document.
//! Uses Chromium support; the load is reported through the engine's lifecycle callbacks.
#![cfg(feature = "e2e-script")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;

use std::time::{Duration, Instant};
use support::{Rig, SESSION, URL};
use vw_browser::{Actor, ReadOptions};
use vw_preview::PageEvent;

/// The page starts loading again with an empty document; `finish_after` later its text arrives and the load ends.
async fn reloading(finish_after: Option<Duration>) -> Rig {
    let rig = Rig::new().await;
    let mount = rig.service.page_mount(SESSION).unwrap();
    let hooks = rig.engine.hooks();
    rig.engine
        .eval("document.title='Front page'; document.body.innerHTML='<script type=\"application/json\">{}</script>'")
        .await;
    assert!(hooks.page_event(&mount.label, PageEvent::Started(URL.parse().unwrap())));
    assert!(rig.service.load_state(SESSION).unwrap().loading);
    if let Some(after) = finish_after {
        let engine = rig.engine.clone();
        tokio::spawn(async move {
            tokio::time::sleep(after).await;
            engine
                .eval("document.body.innerHTML='<h1>Headline</h1><p>' + 'Today\\'s news text. '.repeat(30) + '</p>'")
                .await;
            hooks.page_event(&mount.label, PageEvent::Finished(URL.parse().unwrap()));
        });
    }
    rig
}

#[tokio::test]
async fn read_during_a_reload_answers_the_loaded_page() {
    let rig = reloading(Some(Duration::from_millis(1500))).await;
    let (answer, _) = rig
        .browser
        .read(SESSION, ReadOptions::default(), false, Actor::Agent)
        .await
        .unwrap();
    assert_eq!(answer["loading"], false, "{answer}");
    let chars = answer["untrustedPageContent"]["textChars"]
        .as_i64()
        .unwrap();
    assert!(chars > 200, "read the half-built page: {answer}");
}

#[tokio::test]
async fn read_during_an_endless_load_answers_truthfully_in_bounded_time() {
    let rig = reloading(None).await;
    let start = Instant::now();
    let (answer, _) = rig
        .browser
        .read(SESSION, ReadOptions::default(), false, Actor::Agent)
        .await
        .unwrap();
    assert!(
        start.elapsed() < Duration::from_secs(12),
        "{:?}",
        start.elapsed()
    );
    assert_eq!(answer["loading"], true, "{answer}");
}

#[tokio::test]
async fn read_refuses_a_local_network_page_the_user_opened_during_the_wait() {
    let rig = reloading(None).await;
    let service = rig.service.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(800)).await;
        service
            .open(vw_preview::OpenRequest {
                session_id: SESSION.into(),
                url: "http://10.20.30.40/".parse().unwrap(),
                allowed_ports: vec![],
                downloads_dir: None,
                wait: false,
                lan: true,
            })
            .await
            .unwrap();
    });
    let error = rig
        .browser
        .read(SESSION, ReadOptions::default(), false, Actor::Agent)
        .await
        .unwrap_err();
    assert_eq!(error, vw_browser::LOCAL_NETWORK);
}
