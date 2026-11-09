//! Original-file reads through tool preparation, admission and extraction, without a provider or window.
//! Uses the core fixture and real search adapter; replacements cannot change the bytes in a prepared read.
use super::{document_tools, prepare, search_tool};
use crate::ask::tool_calls::receipts::Execution;
use crate::feature_test_core::Fixture;
use serde_json::json;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

async fn prepared(shared: &Arc<crate::core::Shared>, path: &Path) -> Execution {
    let mut args = json!({"path":path});
    let execution = prepare(shared, "s", "s", "read", "document_read", &mut args)
        .await
        .unwrap();
    let shared = shared.clone();
    execution
        .unwrap_or_else(|| Box::pin(async move { document_tools::read(&shared, "s", &args).await }))
}

fn mode(f: &Fixture, name: &str) {
    f.core
        .shared
        .access
        .observe_request("session/agent/update", "s", &json!({"agentName":name}));
}

async fn search(f: &Fixture) -> String {
    search_tool::call(&f.core.shared, "s", json!({"glob":"**/*.txt"}))
        .await
        .unwrap()
        .as_str()
        .unwrap()
        .to_owned()
}

#[tokio::test]
async fn original_read_creates_no_attachment_and_search_lists_it_once() {
    let f = Fixture::new().await;
    let folder = f.root.join("project");
    f.load("s", &folder, &[], false);
    mode(&f, "auto-approve");
    let path = folder.join("brief.txt");
    std::fs::write(&path, "original text").unwrap();
    let result = prepared(&f.core.shared, &path).await.await.unwrap();
    assert!(result["text"].as_str().unwrap().contains("original text"));
    assert_eq!(search(&f).await, "brief.txt");
    assert!(!folder.join("attachments").exists());
    assert_eq!(result["path"], json!(path));
    f.finish().await;
}

#[tokio::test]
async fn unchanged_original_reuses_digest_cache() {
    let f = Fixture::new().await;
    let folder = f.root.join("project");
    f.load("s", &folder, &[], false);
    mode(&f, "auto-approve");
    let path = folder.join("brief.txt");
    std::fs::write(&path, "cached text").unwrap();
    let first = prepared(&f.core.shared, &path).await.await.unwrap();
    let second = prepared(&f.core.shared, &path).await.await.unwrap();
    assert_eq!(first["cached"], false);
    assert_eq!(second["cached"], true);
    assert_eq!(first["text"], second["text"]);
    assert!(second.get("note").is_none());
    assert!(!folder.join("attachments").exists());
    f.finish().await;
}

#[tokio::test]
async fn changed_original_invalidates_the_cache_without_a_card() {
    let f = Fixture::new().await;
    let folder = f.root.join("project");
    f.load("s", &folder, &[], false);
    let path = folder.join("brief.txt");
    for text in ["first bytes", "changed bytes"] {
        std::fs::write(&path, text).unwrap();
        let read = tokio::time::timeout(Duration::from_secs(1), prepared(&f.core.shared, &path))
            .await
            .expect("local extraction never waits for a card");
        assert_eq!(f.core.asks(), [] as [vw_ask::Ask; 0]);
        let result = read.await.unwrap();
        assert!(result["text"].as_str().unwrap().contains(text));
        assert_eq!(result["cached"], false);
        let repeated = prepared(&f.core.shared, &path).await.await.unwrap();
        assert_eq!(repeated["cached"], true);
    }
    assert!(!folder.join("attachments").exists());
    f.finish().await;
}

#[tokio::test]
async fn replacement_after_preparation_reads_only_checked_bytes() {
    let f = Fixture::new().await;
    let folder = f.root.join("project");
    f.load("s", &folder, &[], false);
    mode(&f, "auto-approve");
    let path = folder.join("brief.txt");
    std::fs::write(&path, "checked bytes").unwrap();
    let read = prepared(&f.core.shared, &path).await;
    std::fs::rename(&path, folder.join("moved.txt")).unwrap();
    std::fs::write(&path, "replacement bytes").unwrap();
    let result = read.await.unwrap();
    assert_eq!(
        result["text"],
        "<file_content>\nFile: brief.txt\n\nchecked bytes\n</file_content>"
    );
    assert!(!folder.join("attachments").exists());
    let next = prepared(&f.core.shared, &path).await.await.unwrap();
    assert_eq!(
        next["text"],
        "<file_content>\nFile: brief.txt\n\nreplacement bytes\n</file_content>"
    );
    f.finish().await;
}

#[cfg(unix)]
#[tokio::test]
async fn symlink_swap_after_preparation_never_reads_outside_bytes() {
    let f = Fixture::new().await;
    let folder = f.root.join("project");
    f.load("s", &folder, &[], false);
    mode(&f, "auto-approve");
    let path = folder.join("brief.txt");
    let outside = f.root.join("outside.txt");
    std::fs::write(&path, "checked bytes").unwrap();
    std::fs::write(&outside, "outside bytes").unwrap();
    let read = prepared(&f.core.shared, &path).await;
    std::fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(outside, &path).unwrap();
    let result = read.await.unwrap();
    assert_eq!(
        result["text"],
        "<file_content>\nFile: brief.txt\n\nchecked bytes\n</file_content>"
    );
    assert!(!folder.join("attachments").exists());
    assert!(
        prepare(
            &f.core.shared,
            "s",
            "s",
            "next",
            "document_read",
            &mut json!({"path":path})
        )
        .await
        .is_err()
    );
    f.finish().await;
}

#[tokio::test]
async fn legacy_copy_remains_readable_and_search_shows_both_paths() {
    let f = Fixture::new().await;
    let folder = f.root.join("project");
    f.load("s", &folder, &[], false);
    mode(&f, "auto-approve");
    let source = folder.join("brief.txt");
    let old = folder.join("attachments/s/brief.txt");
    std::fs::create_dir_all(old.parent().unwrap()).unwrap();
    for path in [&source, &old] {
        std::fs::write(path, "legacy bytes").unwrap();
    }
    let result = prepared(&f.core.shared, &old).await.await.unwrap();
    assert!(result["text"].as_str().unwrap().contains("legacy bytes"));
    assert_eq!(std::fs::read(&old).unwrap(), b"legacy bytes");
    assert_eq!(std::fs::read_dir(old.parent().unwrap()).unwrap().count(), 1);
    assert_eq!(search(&f).await.lines().count(), 2);
    f.finish().await;
}

#[tokio::test]
async fn matching_bytes_at_another_path_reuse_the_cache_without_a_card() {
    let f = Fixture::new().await;
    let folder = f.root.join("project");
    let extra = f.root.join("extra");
    std::fs::create_dir_all(&extra).unwrap();
    f.load("s", &folder, std::slice::from_ref(&extra), false);
    for (index, root) in [folder, extra].iter().enumerate() {
        let path = root.join("brief.txt");
        std::fs::write(&path, "same bytes").unwrap();
        let read = tokio::time::timeout(Duration::from_secs(1), prepared(&f.core.shared, &path))
            .await
            .expect("each local path reads without a card");
        assert_eq!(f.core.asks(), [] as [vw_ask::Ask; 0]);
        let result = read.await.unwrap();
        assert_eq!(result["cached"], index == 1);
    }
    f.finish().await;
}

#[tokio::test]
async fn deleting_a_chat_revokes_prepared_original_bytes() {
    let f = Fixture::new().await;
    let folder = f.root.join("project");
    f.load("s", &folder, &[], false);
    mode(&f, "auto-approve");
    let path = folder.join("brief.txt");
    std::fs::write(&path, "checked bytes").unwrap();
    let read = prepared(&f.core.shared, &path).await;
    f.core.shared.saved_documents.forget("s");
    assert_eq!(read.await.unwrap_err(), "This chat is no longer available.");
    assert_eq!(std::fs::read(&path).unwrap(), b"checked bytes");
    f.finish().await;
}
