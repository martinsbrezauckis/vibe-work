//! Launch-check preservation tests over disposable config files and a Python stdio lobby.
//! Depends on Python 3 and the core dispatcher; no network, desktop or model is used.

// Test fixtures: a failed setup step is a failed test.
#![allow(clippy::unwrap_used)]

use serde_json::json;
use std::sync::Arc;
use vw_gate::{Confirmation, ConfirmationRequest, Confirmer};

struct NoPrompt;
impl Confirmer for NoPrompt {
    async fn confirm(&self, _: &ConfirmationRequest) -> Confirmation {
        panic!("the launch check must not prompt");
    }
}

const LOBBY: &str = r"#!/usr/bin/env python3
import json, os, pathlib, sys
home = pathlib.Path(os.environ['VIBE_HOME'])
for line in sys.stdin:
    frame = json.loads(line)
    message = frame['message']
    if message is None:
        print(json.dumps(frame), flush=True)
        continue
    if 'id' not in message:
        continue
    if message['method'] == 'initialize':
        result = {'serverInfo': {'name': 'model-check-test', 'version': '0'}}
    elif message['method'] == 'session/list':
        result = {'items': [], 'nextCursor': None}
    elif message['method'] == 'config/read':
        (home / 'read-config.toml').write_bytes((home / 'config.toml').read_bytes())
        result = {'config': {'models': json.loads((home / 'rows.json').read_text())}}
    else:
        raise AssertionError(message['method'])
    frame['message'] = {'jsonrpc': '2.0', 'id': message['id'], 'result': result}
    print(json.dumps(frame), flush=True)
";

/// A core over a disposable Vibe home whose `config/read` copies the config it saw to `read-config.toml`.
async fn start(name: &str) -> (crate::Core, std::path::PathBuf, std::path::PathBuf) {
    use std::os::unix::fs::PermissionsExt;

    let root = std::env::temp_dir().join(format!("vw-model-check-{name}-{}", std::process::id()));
    let home = root.join("vibe");
    std::fs::create_dir_all(&home).unwrap();
    let program = root.join("lobby.py");
    std::fs::write(&program, LOBBY).unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
    let core = crate::Core::start(
        vw_supervisor::SupervisorConfig::new(program, home.clone(), root.clone()),
        Arc::new(vw_supervisor::NoSecret),
        NoPrompt,
        crate::CoreConfig {
            instructions: crate::InstructionsConfig {
                cli_home: Some(root.join("cli")),
            },
            ..crate::CoreConfig::default()
        },
    )
    .await
    .unwrap();
    (core, root, home)
}

async fn check(core: &crate::Core) {
    let result = core
        .execute(crate::Origin::Ui, crate::Command::CheckDefaultModel {})
        .await
        .unwrap();
    assert!(matches!(result, crate::CommandResult::Reply { .. }));
}

#[tokio::test]
async fn vibe_rewritten_facts_are_removed_before_the_lobby_reads_an_experiments_off_config() {
    let (core, root, home) = Box::pin(start("rewritten-facts")).await;
    let original = include_str!("fixtures/models_rewritten.toml");
    std::fs::write(home.join("config.toml"), original).unwrap();
    offer(&home, None);
    check(&core).await;
    let read = std::fs::read_to_string(home.join("read-config.toml")).unwrap();
    assert!(
        !read.contains("mistral-large-4-0-3"),
        "orphan facts still reach Vibe: {read}"
    );
    let expected: toml::Table = original.parse().unwrap();
    let actual: toml::Table = read.parse().unwrap();
    assert_eq!(
        actual["models"].as_array().unwrap(),
        &expected["models"].as_array().unwrap()[..1]
    );
    check(&core).await;
    assert_eq!(
        std::fs::read_to_string(home.join("config.toml")).unwrap(),
        read
    );
    core.shutdown().await;
    std::fs::remove_dir_all(root).unwrap();
}

/// Exact facts remain ours after Vibe rewrites the file, even when the side record is stale.
#[tokio::test]
async fn an_identical_unmarked_rewrite_is_removed_with_a_stale_record() {
    // Boxed (clippy large_futures): the start future exceeds 16 KiB on aarch64.
    let (core, root, home) = Box::pin(start("replacement")).await;
    let file = home.join("config.toml");
    std::fs::write(&file, "active_model = 'large-4'\n").unwrap();
    offer(&home, Some("mistral-large-4-0"));
    check(&core).await;
    assert!(crate::model_facts::recorded(&home).is_some());
    let replacement = std::fs::read_to_string(&file)
        .unwrap()
        .replace(crate::model_facts::MARK.trim_end(), "");
    std::fs::write(&file, &replacement).unwrap();
    offer(&home, None);
    check(&core).await;
    let bare = crate::model_facts::without(&replacement, None).unwrap();
    assert_eq!(std::fs::read_to_string(&file).unwrap(), bare);
    assert_eq!(
        std::fs::read_to_string(home.join("read-config.toml")).unwrap(),
        bare
    );
    assert!(crate::model_facts::recorded(&home).is_none());
    core.shutdown().await;
    std::fs::remove_dir_all(root).unwrap();
}

/// The account's model list with Large 4 under `alias`, or (`None`) an account list that no longer offers Large 4.
/// "Not offered" keeps another account model in the list; a separate case covers a settled empty account list.
fn offer(home: &std::path::Path, alias: Option<&str>) {
    let rows = alias.map_or_else(
        || {
            json!([{"alias": "mistral-medium-3", "name": "mistral-medium-3",
                "source": "growthbook", "displayName": "Mistral Medium 3"}])
        },
        |alias| {
            json!([{"alias": alias, "name": "mistral-large-4-0",
                "source": "growthbook", "displayName": "Mistral Large 4"}])
        },
    );
    std::fs::write(home.join("rows.json"), rows.to_string()).unwrap();
}

#[tokio::test]
async fn a_different_alias_with_our_prices_survives_the_launch_check_byte_for_byte() {
    // Boxed (clippy large_futures): the start future exceeds 16 KiB on aarch64.
    let (core, root, home) = Box::pin(start("alias")).await;
    let file = home.join("config.toml");
    for offered in [false, true] {
        offer(&home, offered.then_some("mistral-large-4-0-3"));
        for window in ["", "max_context_length = 1048576\n"] {
            let original = format!(
                "# My prices\n[[models]]\nalias = 'my-model'\n{window}input_price = 0.70 # keep this\ncached_input_price = 0.07\noutput_price = 2.10\n"
            );
            std::fs::write(&file, &original).unwrap();
            for _ in 0..2 {
                check(&core).await;
                let after = std::fs::read_to_string(&file).unwrap();
                let read = std::fs::read_to_string(home.join("read-config.toml")).unwrap();
                assert!(
                    read.starts_with(&original),
                    "the lobby read lost user bytes: {read:?}"
                );
                if offered {
                    assert_eq!(
                        crate::model_facts::without(&after, None).as_deref(),
                        Some(original.as_str())
                    );
                } else {
                    assert_eq!(after, original, "the launch check changed user bytes");
                }
            }
        }
    }
    // A settled read without account models must remove our marked facts row too: it is an orphan.
    let (original, written) = crate::model_facts::with(
        "# My own line\nactive_model = 'large-4'\n",
        "mistral-large-4-0-3",
        &json!({}),
    );
    // Installed as the launch check installs it: the entry and its record together.
    crate::model_facts::record(&home, written.as_ref()).unwrap();
    std::fs::write(home.join("rows.json"), "[]").unwrap();
    std::fs::write(&file, &original).unwrap();
    core.execute(crate::Origin::Ui, crate::Command::CheckDefaultModel {})
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        crate::model_facts::without(&original, None).unwrap(),
        "a settled empty account list kept an orphan override"
    );
    assert!(crate::model_facts::recorded(&home).is_none());
    core.shutdown().await;
    std::fs::remove_dir_all(root).unwrap();
}

/// A person's entry with extra settings belongs to them, even on the same alias and with our prices.
/// Every launch check leaves it byte for byte, offered or not, and never adds a second entry for that alias
/// (Vibe keeps only the last entry of an alias, which would replace theirs).
#[tokio::test]
async fn a_persons_same_alias_entry_with_other_content_survives_the_launch_check_byte_for_byte() {
    // Boxed (clippy large_futures): the start future exceeds 16 KiB on aarch64.
    let (core, root, home) = Box::pin(start("same-alias")).await;
    let file = home.join("config.toml");
    for alias in [
        "mistral-large-4",
        "mistral-large-4-0",
        "mistral-large-4-0-3",
    ] {
        for offered in [false, true] {
            offer(&home, offered.then_some(alias));
            for window in ["", "max_context_length = 1048576\n"] {
                for spelling in ["0.70", "0.7"] {
                    let original = format!(
                        "active_model = \"large-4\"\n\n[[models]]\nalias = \"{alias}\"\n{window}input_price = {spelling}\ncached_input_price = 0.07\noutput_price = 2.1\nmax_output_tokens = 1000\n"
                    );
                    std::fs::write(&file, &original).unwrap();
                    for _ in 0..2 {
                        check(&core).await;
                        let read = std::fs::read_to_string(home.join("read-config.toml")).unwrap();
                        assert_eq!(read, original, "{alias}: the lobby read lost the entry");
                        let after = std::fs::read_to_string(&file).unwrap();
                        assert_eq!(
                            after, original,
                            "{alias}, offered {offered}: the launch check changed the person's entry"
                        );
                    }
                }
            }
        }
    }
    core.shutdown().await;
    std::fs::remove_dir_all(root).unwrap();
}

/// Vibe strips comments: refresh our exact entry while offered and remove it when retired.
#[tokio::test]
async fn our_entry_that_lost_its_mark_is_refreshed_or_removed_without_a_record() {
    // Boxed (clippy large_futures): the start future exceeds 16 KiB on aarch64.
    let (core, root, home) = Box::pin(start("record")).await;
    let file = home.join("config.toml");
    let base = "active_model = \"large-4\"\n";
    for alias in [
        "mistral-large-4",
        "mistral-large-4-0",
        "mistral-large-4-0-3",
    ] {
        std::fs::write(&file, base).unwrap();
        offer(&home, Some(alias));
        check(&core).await;
        let marked = std::fs::read_to_string(&file).unwrap();
        assert_eq!(
            crate::model_facts::without(&marked, None).as_deref(),
            Some(base),
            "{marked}"
        );
        // What Vibe's own rewrite leaves of it: the same entry without its comment.
        let unmarked = marked.replace(crate::model_facts::MARK.trim_end(), "");
        std::fs::write(&file, &unmarked).unwrap();
        check(&core).await;
        let again = std::fs::read_to_string(&file).unwrap();
        assert_eq!(again.matches(alias).count(), 1, "{again}");
        assert_eq!(
            crate::model_facts::without(&again, None).as_deref(),
            Some(base),
            "{again}"
        );
        std::fs::write(
            &file,
            again.replace(crate::model_facts::MARK.trim_end(), ""),
        )
        .unwrap();
        offer(&home, None);
        check(&core).await;
        assert_eq!(std::fs::read_to_string(&file).unwrap(), base, "{alias}");
        assert!(crate::model_facts::recorded(&home).is_none());
        // Recognition does not require the side record on a later launch either.
        std::fs::write(&file, &unmarked).unwrap();
        check(&core).await;
        assert_eq!(std::fs::read_to_string(&file).unwrap(), base, "{alias}");
    }
    core.shutdown().await;
    std::fs::remove_dir_all(root).unwrap();
}

/// The first locked launch after an update cannot use the empty account list as proof of retirement.
#[tokio::test]
async fn locked_update_keeps_marked_and_rewritten_facts_until_account_models_can_be_read() {
    let (core, _root, home) = Box::pin(start("locked-update")).await;
    let file = home.join("config.toml");
    let (marked, written) = crate::model_facts::with(
        "active_model = 'large-4'\n",
        "mistral-large-4-0-3",
        &json!({}),
    );
    for original in [
        marked.clone(),
        marked.replace(crate::model_facts::MARK.trim_end(), ""),
    ] {
        for rows in [json!([]), json!([{"alias":"large-4", "source":"config"}])] {
            crate::model_facts::record(&home, written.as_ref()).unwrap();
            std::fs::write(&file, &original).unwrap();
            std::fs::write(home.join("rows.json"), rows.to_string()).unwrap();
            let (_, current) = super::bare_lobby(&core.shared, &home, &file, false)
                .await
                .unwrap();
            assert_eq!(
                std::fs::read_to_string(&file).unwrap(),
                original,
                "a locked launch lost the saved context window and prices"
            );
            assert_eq!(current, original);
            assert_eq!(crate::model_facts::recorded(&home), written);
            // Once unlocked, a populated account list can establish that Large 4 was retired.
            offer(&home, None);
            check(&core).await;
            assert!(
                !std::fs::read_to_string(&file)
                    .unwrap()
                    .contains("mistral-large-4-0-3")
            );
        }
    }
    core.shutdown().await;
}
