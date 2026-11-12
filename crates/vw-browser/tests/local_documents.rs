//! Opens a real local HTML file through the production preview grant and navigation guard.
//! The Chromium fixture checks actual document text and refuses other file paths before and after navigation.
#![cfg(feature = "e2e-script")]
#![allow(clippy::unwrap_used)]
mod support;
use support::{Rig, SESSION};
use url::Url;
use vw_preview::OpenRequest;

#[tokio::test]
async fn selected_local_document_opens_and_other_files_remain_refused() {
    let rig = Rig::new().await;
    rig.engine.public_pages().await;
    let path = rig.dir.join("article.html");
    std::fs::write(
        &path,
        "<!doctype html><title>Local article</title><h1>Local content</h1>",
    )
    .unwrap();
    let url = Url::from_file_path(&path).unwrap();
    // The fixture has one Chromium surface; release its initial page before mounting the file tab.
    rig.service.close(SESSION).await.unwrap();
    let page = rig.service.create_web_tab(SESSION).unwrap();
    let request = || OpenRequest {
        session_id: page.clone(),
        url: url.clone(),
        allowed_ports: vec![],
        downloads_dir: None,
        wait: true,
        lan: false,
    };
    assert!(rig.service.open(request()).await.is_err());
    rig.service.allow_local_document(&page, &url).unwrap();
    let view = rig.service.open(request()).await.unwrap();
    assert!(
        serde_json::to_string(&view)
            .unwrap()
            .contains("article.html")
    );
    assert_eq!(
        rig.engine
            .eval("document.querySelector('h1').textContent")
            .await,
        "Local content"
    );
    let other = Url::from_file_path(rig.dir.join("other.html")).unwrap();
    let mut changed = request();
    changed.url = other;
    assert!(rig.service.open(changed).await.is_err());
    assert!(rig.service.allow_local_document(SESSION, &url).is_err());
}
