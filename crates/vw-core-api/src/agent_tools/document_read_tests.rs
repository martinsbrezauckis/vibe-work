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

#[cfg(windows)]
#[tokio::test]
async fn windows_search_and_document_read_allow_both_ordinary_hard_link_names() {
    use std::os::windows::fs::OpenOptionsExt;

    let f = Fixture::new().await;
    let folder = f.root.join("project");
    f.load("s", &folder, &[], false);
    f.load("second", &folder, &[], false);
    let original = folder.join("a.txt");
    let alias = folder.join("b.txt");
    let plain = folder.join("plain.txt");
    std::fs::write(&original, "WINSEARCH_ADAPTER_MARKER").unwrap();
    std::fs::hard_link(&original, &alias).unwrap();
    std::fs::write(&plain, "WINSEARCH_PLAIN_MARKER").unwrap();
    let protected = f.root.join("app-data/venv/protected.txt");
    std::fs::create_dir_all(protected.parent().unwrap()).unwrap();
    std::fs::write(&protected, "WINSEARCH_PROTECTED_MARKER").unwrap();
    let protected_alias = folder.join("protected.txt");
    std::fs::hard_link(&protected, &protected_alias).unwrap();
    let held_path = f.root.join("app-data/venv/held.db");
    std::fs::write(&held_path, "held app state").unwrap();
    let held = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(0)
        .open(&held_path)
        .unwrap();
    assert!(std::fs::File::open(&held_path).is_err());
    assert!(vw_platform::final_path::open_metadata(&held_path).is_ok());
    for selected in [&original, &alias] {
        println!(
            "WINSEARCH_ADAPTER selected={} reported={}",
            selected.display(),
            vw_platform::final_path::final_path(&std::fs::File::open(selected).unwrap())
                .unwrap()
                .display()
        );
    }
    for name in ["ask", "accept-edits", "auto-approve"] {
        mode(&f, name);
        let args = json!({"query":"WINSEARCH_"});
        let (first, second) = tokio::join!(
            search_tool::call(&f.core.shared, "s", args.clone()),
            search_tool::call(&f.core.shared, "second", args),
        );
        for answer in [first, second] {
            let text = answer.unwrap();
            let text = text.as_str().unwrap();
            assert!(
                text.contains("a.txt:1: WINSEARCH_ADAPTER_MARKER"),
                "{name}: {text}"
            );
            assert!(
                text.contains("b.txt:1: WINSEARCH_ADAPTER_MARKER"),
                "{name}: {text}"
            );
            assert!(
                text.contains("plain.txt:1: WINSEARCH_PLAIN_MARKER"),
                "{name}: {text}"
            );
            assert!(!text.contains("PROTECTED"), "{name}: {text}");
        }
        for selected in [&original, &alias, &plain] {
            let read = prepared(&f.core.shared, selected).await.await.unwrap();
            assert!(
                read["text"]
                    .as_str()
                    .unwrap()
                    .contains(if selected == &plain {
                        "WINSEARCH_PLAIN_MARKER"
                    } else {
                        "WINSEARCH_ADAPTER_MARKER"
                    })
            );
        }
        assert!(
            prepare(
                &f.core.shared,
                "s",
                "s",
                "protected",
                "document_read",
                &mut json!({"path":protected_alias})
            )
            .await
            .is_err()
        );
    }
    windows_invalid_inputs_are_refused(&f).await;
    drop(held);
    f.finish().await;
}

#[cfg(windows)]
async fn windows_invalid_inputs_are_refused(f: &Fixture) {
    for path in ["", "missing.txt"] {
        assert!(
            prepare(
                &f.core.shared,
                "s",
                "s",
                "missing",
                "document_read",
                &mut json!({"path":path})
            )
            .await
            .is_err()
        );
    }
    assert!(
        search_tool::call(&f.core.shared, "s", json!({"query":""}))
            .await
            .is_err()
    );
    assert!(
        search_tool::call(&f.core.shared, "missing", json!({"query":"WINSEARCH_"}))
            .await
            .is_err()
    );
}

#[cfg(unix)]
#[tokio::test]
async fn realistic_hard_links_work_in_all_access_modes_and_parallel_chats() {
    let f = Fixture::new().await;
    let cache = f.root.join("app-data/venv");
    std::fs::create_dir_all(&cache).unwrap();
    for index in 0..15_000 {
        std::fs::write(cache.join(format!("package-{index}.py")), "single link").unwrap();
    }
    let folder = f.root.join("project");
    f.load("s", &folder, &[], false);
    f.load("second", &folder, &[], false);
    let original = folder.join("a.txt");
    let alias = folder.join("b.txt");
    std::fs::write(&original, "HARDLINKS3_ADAPTER_MARKER").unwrap();
    std::fs::hard_link(&original, &alias).unwrap();
    let protected = cache.join("protected.txt");
    std::fs::write(&protected, "HARDLINKS3_PROTECTED_MARKER").unwrap();
    let protected_alias = folder.join("protected.txt");
    std::fs::hard_link(&protected, &protected_alias).unwrap();
    for name in ["ask", "accept-edits", "auto-approve"] {
        mode(&f, name);
        let args = json!({"query":"HARDLINKS3_"});
        let (first, second) = tokio::join!(
            search_tool::call(&f.core.shared, "s", args.clone()),
            search_tool::call(&f.core.shared, "second", args),
        );
        for answer in [first, second] {
            let answer = answer.unwrap();
            let text = answer.as_str().unwrap();
            assert!(
                text.contains("a.txt:1: HARDLINKS3_ADAPTER_MARKER"),
                "{name}: {text}"
            );
            assert!(
                text.contains("b.txt:1: HARDLINKS3_ADAPTER_MARKER"),
                "{name}: {text}"
            );
            assert!(!text.contains("PROTECTED"), "{name}: {text}");
        }
        let read = prepared(&f.core.shared, &alias).await.await.unwrap();
        assert!(
            read["text"]
                .as_str()
                .unwrap()
                .contains("HARDLINKS3_ADAPTER_MARKER")
        );
        assert!(
            prepare(
                &f.core.shared,
                "s",
                "s",
                "negative",
                "document_read",
                &mut json!({"path":protected_alias})
            )
            .await
            .is_err()
        );
        assert_eq!(f.core.asks(), [] as [vw_ask::Ask; 0]);
        println!(
            "HARDLINKS3 adapters: {name}: parallel search + document positive/negative passed"
        );
    }
    for path in ["", "missing.txt"] {
        assert!(
            prepare(
                &f.core.shared,
                "s",
                "s",
                "missing",
                "document_read",
                &mut json!({"path":path})
            )
            .await
            .is_err()
        );
    }
    assert!(
        search_tool::call(&f.core.shared, "s", json!({"query":""}))
            .await
            .is_err()
    );
    assert!(
        search_tool::call(&f.core.shared, "missing", json!({"query":"HARDLINKS3_"}))
            .await
            .is_err()
    );
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
