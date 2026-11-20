//! Push card, execution and receipt boundaries with disposable local bare repositories.
//! PREDICT: each card permits one exact push; Full access produces a visible receipt without a card.
use serde_json::{Value, json};
use std::{path::Path, process::Command, time::Duration};

fn git(root: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(root)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().into()
}

async fn fixture() -> crate::feature_test_core::Fixture {
    let f = crate::feature_test_core::Fixture::new().await;
    vw_changes::prepare_root(&f.root.join("app-data/changes")).unwrap();
    let root = f.root.join("project");
    std::fs::create_dir_all(&root).unwrap();
    git(&root, &["init", "-b", "main"]);
    git(&root, &["config", "user.name", "Test"]);
    git(&root, &["config", "user.email", "test@example.invalid"]);
    git(&root, &["commit", "--allow-empty", "-m", "initial"]);
    git(&f.root, &["init", "--bare", "remote.git"]);
    git(
        &root,
        &[
            "remote",
            "add",
            "origin",
            f.root.join("remote.git").to_str().unwrap(),
        ],
    );
    f.load("chat", &root, &[], true);
    f.load("other", &root, &[], true);
    f.core.shared.developer_permissions.set_for_test(true);
    f
}

fn mode(f: &crate::feature_test_core::Fixture, chat: &str, mode: &str) {
    f.core
        .shared
        .access
        .observe_request("session/agent/update", chat, &json!({"agentName":mode}));
}

async fn prepare(
    f: &crate::feature_test_core::Fixture,
    chat: &str,
    args: Value,
) -> Result<crate::ask::tool_calls::receipts::Execution, String> {
    super::prepare(
        &f.core.shared,
        chat,
        chat,
        "push-call",
        "push",
        &mut args.clone(),
    )
    .await?
    .ok_or_else(|| "push has no prepared app action".into())
}

fn history(f: &crate::feature_test_core::Fixture, chat: &str) -> Value {
    let mut state = json!({"history":[]});
    crate::ask::tool_calls::overlay(&f.core.shared, chat, &mut state);
    state["history"].clone()
}

fn settle(f: &crate::feature_test_core::Fixture, id: &str, decision: &str) {
    crate::ask::tool_calls::answer_by(
        &f.core.shared,
        crate::Origin::Ui,
        "chat",
        id.strip_prefix("callback:").unwrap(),
        &json!({"decision":{"type":decision}}),
        vw_ask::AnsweredBy::Person,
    )
    .unwrap();
}

#[tokio::test]
async fn push_ask_and_accept_edits_have_once_only_cards_then_push() {
    for access in ["ask", "accept-edits"] {
        let f = fixture().await;
        mode(&f, "chat", access);
        let shared = f.core.shared.clone();
        let task = tokio::spawn(async move {
            super::prepare(&shared, "chat", "chat", "card-call", "push", &mut json!({})).await
        });
        tokio::time::timeout(Duration::from_secs(10), async {
            while f.core.asks().is_empty() && !task.is_finished() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert!(!task.is_finished(), "{access}: no push card");
        let ask = f.core.asks().into_iter().next().unwrap();
        assert_eq!(ask.offers, [vw_ask::Choice::Once, vw_ask::Choice::No]);
        let cards = history(&f, "chat");
        let facts = &cards[0]["detail"]["effect"]["vwPushPlan"];
        assert_eq!(facts["remote"], "origin");
        assert_eq!(facts["branch"], "main");
        assert_eq!(facts["commits"], 1);
        assert!(facts["url"].as_str().unwrap().ends_with("remote.git"));
        assert_eq!(history(&f, "other"), json!([]));
        f.core
            .answer_ask(
                &ask.id,
                vw_ask::Answer {
                    answer: vw_ask::Choice::Session,
                    note: None,
                },
            )
            .await
            .unwrap_err();
        settle(&f, &ask.id, "approve");
        let execution = task.await.unwrap().unwrap().expect("prepared push");
        let answer = execution.await.unwrap();
        assert_eq!(answer["status"], "pushed");
        assert_eq!(
            git(
                &f.root.join("remote.git"),
                &["rev-parse", "refs/heads/main"]
            ),
            answer["commit"].as_str().unwrap()
        );
        assert_eq!(f.core.asks(), [] as [vw_ask::Ask; 0]);
        assert_eq!(f.core.ask_session_grants(), [] as [vw_ask::SessionGrant; 0]);
        f.finish().await;
    }
}

#[tokio::test]
async fn push_full_access_has_a_chat_notice_and_returns_the_exact_commit() {
    let f = fixture().await;
    mode(&f, "chat", "auto-approve");
    let answer = prepare(&f, "chat", json!({})).await.unwrap().await.unwrap();
    assert_eq!(f.core.asks(), [] as [vw_ask::Ask; 0]);
    assert_eq!(answer["remoteBranch"], "main");
    let notices = history(&f, "chat");
    assert_eq!(notices[0]["type"], "notice");
    let text = notices[0]["message"].as_str().unwrap();
    assert!(text.contains("origin/main") && text.contains(answer["commit"].as_str().unwrap()));
    assert_eq!(history(&f, "other"), json!([]));
    f.core.shared.developer_permissions.set_for_test(false);
    mode(&f, "chat", "ask");
    crate::ask::tool_calls::permission(
        &f.core.shared,
        "chat",
        "chat",
        "collect",
        "push",
        &json!({"pushId":"latest"}),
    )
    .await
    .unwrap();
    let collected = prepare(&f, "chat", json!({"pushId":"latest"}))
        .await
        .unwrap()
        .await
        .unwrap();
    assert_eq!(collected, answer);
    assert_eq!(f.core.asks(), [] as [vw_ask::Ask; 0]);
    let other = prepare(&f, "other", json!({"pushId":"latest"}))
        .await
        .unwrap()
        .await
        .unwrap_err();
    assert!(other.contains("No push job"));
    f.finish().await;
}

#[tokio::test]
async fn push_missing_chat_repository_remote_and_head_refuse_without_cards() {
    let f = fixture().await;
    mode(&f, "chat", "auto-approve");
    assert!(prepare(&f, "closed", json!({})).await.is_err());
    let plain = f.root.join("plain");
    f.load("plain", &plain, &[], true);
    mode(&f, "plain", "auto-approve");
    assert!(prepare(&f, "plain", json!({})).await.is_err());
    git(&f.root.join("project"), &["remote", "remove", "origin"]);
    assert!(prepare(&f, "chat", json!({})).await.is_err());
    git(
        &f.root.join("project"),
        &[
            "remote",
            "add",
            "origin",
            f.root.join("remote.git").to_str().unwrap(),
        ],
    );
    git(&f.root.join("project"), &["checkout", "--detach"]);
    assert!(prepare(&f, "chat", json!({})).await.is_err());
    let answer = prepare(&f, "chat", json!({"branch":"main"}))
        .await
        .unwrap()
        .await
        .unwrap();
    assert_eq!(answer["remoteBranch"], "main");
    assert_eq!(f.core.asks(), [] as [vw_ask::Ask; 0]);
    f.finish().await;
}

#[tokio::test]
async fn push_full_chat_can_finish_while_a_second_chat_waits_on_its_own_card() {
    let f = fixture().await;
    mode(&f, "chat", "ask");
    mode(&f, "other", "auto-approve");
    let shared = f.core.shared.clone();
    let waiting = tokio::spawn(async move {
        super::prepare(&shared, "chat", "chat", "waiting", "push", &mut json!({})).await
    });
    tokio::time::timeout(Duration::from_secs(10), async {
        while f.core.asks().is_empty() && !waiting.is_finished() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert!(!waiting.is_finished());
    let ask = f.core.asks().into_iter().next().unwrap();
    let pushed = prepare(&f, "other", json!({}))
        .await
        .unwrap()
        .await
        .unwrap();
    assert_eq!(pushed["status"], "pushed");
    assert!(
        !waiting.is_finished(),
        "another chat cannot answer this card"
    );
    assert_eq!(f.core.asks().len(), 1);
    assert!(
        history(&f, "chat")
            .as_array()
            .unwrap()
            .iter()
            .all(|entry| entry["type"] != "notice")
    );
    assert_eq!(history(&f, "other")[0]["type"], "notice");
    settle(&f, &ask.id, "deny");
    assert!(waiting.await.unwrap().is_err());
    f.finish().await;
}

#[tokio::test]
async fn push_rechecks_switch_mode_tip_and_input_before_execution() {
    let f = fixture().await;
    mode(&f, "chat", "auto-approve");
    for input in [
        json!({"force":true}),
        json!({"branch":""}),
        json!({"remote":""}),
        json!({"branch":123}),
        json!({"branch":"missing"}),
        json!({"remote":"missing"}),
        json!({"remote":null}),
        json!({"pushId":null}),
        json!({"pushId":""}),
        json!({"pushId":"latest","branch":"main"}),
    ] {
        assert!(prepare(&f, "chat", input).await.is_err());
    }
    let execution = prepare(&f, "chat", json!({})).await.unwrap();
    f.core.shared.developer_permissions.set_for_test(false);
    assert!(
        execution
            .await
            .unwrap_err()
            .contains("Developer permissions")
    );
    f.core.shared.developer_permissions.set_for_test(true);
    let execution = prepare(&f, "chat", json!({})).await.unwrap();
    mode(&f, "chat", "ask");
    assert!(execution.await.unwrap_err().contains("access mode changed"));
    mode(&f, "chat", "auto-approve");
    let execution = prepare(&f, "chat", json!({})).await.unwrap();
    git(
        &f.root.join("project"),
        &["commit", "--allow-empty", "-m", "changed"],
    );
    assert!(execution.await.unwrap_err().contains("changed"));
    assert_eq!(history(&f, "chat"), json!([]));
    f.finish().await;
}

#[tokio::test]
async fn push_non_fast_forward_is_reported_and_never_forced() {
    let f = fixture().await;
    mode(&f, "chat", "auto-approve");
    prepare(&f, "chat", json!({})).await.unwrap().await.unwrap();
    let root = f.root.join("project");
    git(&root, &["checkout", "-b", "competitor"]);
    git(&root, &["commit", "--allow-empty", "-m", "remote advance"]);
    let other = git(&root, &["rev-parse", "HEAD"]);
    git(&root, &["push", "--no-verify", "origin", "competitor:main"]);
    git(&root, &["checkout", "main"]);
    git(&root, &["commit", "--allow-empty", "-m", "divergent local"]);
    let refusal = prepare(&f, "chat", json!({}))
        .await
        .unwrap()
        .await
        .unwrap_err();
    assert!(
        refusal.contains("non-fast-forward") || refusal.contains("moved on"),
        "{refusal}"
    );
    assert_eq!(
        git(
            &f.root.join("remote.git"),
            &["rev-parse", "refs/heads/main"]
        ),
        other
    );
    assert_eq!(
        history(&f, "chat").as_array().unwrap().len(),
        1,
        "failure has no success notice"
    );
    f.finish().await;
}
