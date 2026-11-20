//! A chat that picked Vibe Work's retired `large-4` alias reopens and sends on the catalog Large 4 with its own
//! thinking level, whatever the Settings default is, without a notice; a model that is really gone still tells the
//! person. Uses a Python stdio stand-in for Vibe (config/read, runtime/read, config/model/write) and real private
//! record files; no network, desktop or model.
//!
//! PREDICT (written before the change): Vibe resumes the chat on the Settings default (Medium here) and keeps its own
//! "Model 'large-4' is not available" warning. Core then reads the saved `large-4` choice as unavailable, applies the
//! Settings default (Medium) and raises the "unavailable right now, so this chat now uses your default model" alert;
//! the saved choice becomes `mistral-medium-latest`, so the next plain Send runs on Medium too.

// Test fixtures: a failed setup step is a failed test.
#![allow(clippy::unwrap_used)]

use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use vw_gate::{Confirmation, ConfirmationRequest, Confirmer};

struct NoPrompt;
impl Confirmer for NoPrompt {
    async fn confirm(&self, _: &ConfirmationRequest) -> Confirmation {
        panic!("restoring a chat's model must not prompt");
    }
}

/// Vibe's side: one chat whose runtime starts on `state.json` and follows each session-only model write; the model
/// list is read from `models.json` at each call.
const VIBE: &str = r"#!/usr/bin/env python3
import json, os, pathlib, sys
home = pathlib.Path(os.environ['VIBE_HOME'])
def listed(project=False):
    path = home / ('project-models.json' if project and (home / 'project-models.json').exists() else 'models.json')
    return json.loads(path.read_text())
def state():
    return json.loads((home / 'state.json').read_text())
def runtime():
    current = state()
    rows = listed(current.get('project', False))
    row = dict(next(row for row in rows if row['alias'] == current['active']))
    row['thinking'] = current['thinking']
    return {'config': {'models': rows, 'activeModel': row,
                       'validationWarnings': current['warnings']}}
for line in sys.stdin:
    frame = json.loads(line)
    message = frame['message']
    if message is None:
        print(json.dumps(frame), flush=True)
        continue
    if 'id' not in message:
        continue
    method = message['method']
    params = message.get('params') or {}
    if method == 'initialize':
        result = {'serverInfo': {'name': 'retired-choice-test', 'version': '0'}}
    elif method == 'session/list':
        result = {'items': [], 'nextCursor': None}
    elif method == 'config/read':
        rows = listed(bool(params.get('cwd')))
        default = dict(next(row for row in rows if row['alias'] == state()['default']))
        result = {'config': {'models': rows, 'activeModel': default, 'validationWarnings': []}}
    elif method == 'runtime/read':
        result = {'runtime': runtime()}
    elif method == 'config/model/write':
        current = state()
        if any(row['alias'] == params['modelAlias'] for row in listed(current.get('project', False))):
            current['active'] = params['modelAlias']
            current['thinking'] = params['reasoningEffort']
            current['warnings'] = []
        current['writes'].append([params['modelAlias'], params['reasoningEffort']])
        (home / 'state.json').write_text(json.dumps(current))
        result = {'runtime': runtime(), 'failures': []}
    else:
        raise AssertionError(method)
    frame['message'] = {'jsonrpc': '2.0', 'id': message['id'], 'result': result}
    print(json.dumps(frame), flush=True)
";

const CHAT: &str = "chat-large-4";

fn models() -> Value {
    json!([
        {"alias":"mistral-large-4-0","name":"mistral-large-4-0","source":"catalog",
         "displayName":"Mistral Large 4","thinking":"high","thinkingLevels":["low","medium","high"]},
        {"alias":"mistral-medium-latest","name":"mistral-medium-latest","source":"catalog",
         "displayName":"Mistral Medium 3.5","thinking":"high","thinkingLevels":["off","high"]}
    ])
}

struct Run {
    core: crate::Core,
    root: PathBuf,
    home: PathBuf,
    sessions: PathBuf,
}

/// A core whose saved record for [`CHAT`] is `saved` and whose Vibe resumed the chat on the Settings default
/// (Medium), warning that `pin` is not available.
async fn start(name: &str, saved: &Value, pin: &str) -> Run {
    use std::os::unix::fs::PermissionsExt;

    let base = std::env::var_os("VW_TEST_ROOT").map_or_else(std::env::temp_dir, PathBuf::from);
    let root = base.join(format!("vw-retired-choice-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let home = root.join("vibe");
    let sessions = root.join("app-sessions");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&sessions).unwrap();
    std::fs::write(home.join("models.json"), models().to_string()).unwrap();
    let warning = format!(
        "Model '{pin}' is not available — running 'Mistral Medium 3.5'. Pick another model to stop this notice."
    );
    std::fs::write(
        home.join("state.json"),
        json!({"active":"mistral-medium-latest","default":"mistral-medium-latest","thinking":"high",
            "warnings":[warning],"writes":[]})
            .to_string(),
    )
    .unwrap();
    std::fs::write(
        sessions.join(super::records::FILE),
        json!({"version":1,"chats":{CHAT: saved}}).to_string(),
    )
    .unwrap();
    let program = root.join("vibe.py");
    std::fs::write(&program, VIBE).unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
    // Boxed (clippy large_futures): the start future exceeds 16 KiB on aarch64.
    let core = Box::pin(crate::Core::start(
        vw_supervisor::SupervisorConfig::new(program, home.clone(), root.clone()),
        Arc::new(vw_supervisor::NoSecret),
        NoPrompt,
        crate::CoreConfig {
            instructions: crate::InstructionsConfig {
                cli_home: Some(root.join("cli")),
            },
            sessions_dir: Some(sessions.clone()),
            ..crate::CoreConfig::default()
        },
    ))
    .await
    .unwrap();
    Run {
        core,
        root,
        home,
        sessions,
    }
}

impl Run {
    fn state(&self) -> Value {
        serde_json::from_slice(&std::fs::read(self.home.join("state.json")).unwrap()).unwrap()
    }

    fn saved(&self) -> Value {
        let file = std::fs::read(self.sessions.join(super::records::FILE)).unwrap();
        serde_json::from_slice::<Value>(&file).unwrap()["chats"][CHAT].clone()
    }

    /// What Vibe's resume left: the chat's runtime before Core restores the choice.
    async fn resumed(&self) -> Value {
        super::restore::read(&self.core.shared, CHAT).await.unwrap()
    }

    async fn finish(self) {
        self.core.shutdown().await;
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}

async fn alerts(events: &mut crate::CoreSubscription) -> Vec<String> {
    let mut seen = Vec::new();
    while let Ok(Some(event)) =
        tokio::time::timeout(Duration::from_millis(300), events.next()).await
    {
        if let crate::CoreEvent::Alert { message, .. } = event {
            seen.push(message);
        }
    }
    seen
}

fn active(runtime: &Value) -> (String, String) {
    let model = &runtime["config"]["activeModel"];
    (
        model["alias"].as_str().unwrap_or_default().into(),
        model["thinking"].as_str().unwrap_or_default().into(),
    )
}

fn large_4(effort: &str) -> (String, String) {
    ("mistral-large-4-0".into(), effort.into())
}

fn saved_large_4(effort: &str) -> Value {
    json!({"remembered":{"alias":"large-4","effort":effort,"display_name":"Mistral Large 4","name":"mistral-large-4"}})
}

/// A chat that picked Large 4 by the retired alias (medium thinking here, so its own level is told apart from the
/// default's high) while the Settings default is Medium: reopen and a plain Send keep Large 4.
#[tokio::test]
async fn a_chat_pinned_to_retired_large_4_keeps_catalog_large_4_over_a_different_default() {
    let run = Box::pin(start("discriminator", &saved_large_4("medium"), "large-4")).await;
    let shared = run.core.shared.clone();
    let mut events = run.core.subscribe(Some(CHAT.into()));
    let pin = super::recorded(&json!({"model":"large-4","reasoningEffort":"medium"}));

    let last = run.resumed().await;
    let reopened = super::restore(&shared, CHAT, last, pin).await;
    assert_eq!(active(&reopened), large_4("medium"), "{reopened}");
    assert_eq!(
        reopened["config"]["validationWarnings"],
        json!([]),
        "Vibe's own missing-model warning is cleared by the resolved pin"
    );
    let reopen_alerts = alerts(&mut events).await;
    assert!(
        reopen_alerts.is_empty(),
        "no notice at reopen: {reopen_alerts:?}"
    );
    let saved = run.saved();
    assert_eq!(saved["remembered"]["alias"], "mistral-large-4-0", "{saved}");
    assert_eq!(saved["remembered"]["effort"], "medium", "{saved}");
    assert_eq!(
        saved["remembered"]["display_name"], "Mistral Large 4",
        "{saved}"
    );

    // A plain Send without touching the picker.
    super::before(&shared, "turn/start", &json!({"sessionId":CHAT})).await;
    let state = run.state();
    assert_eq!(state["active"], "mistral-large-4-0", "{state}");
    assert_eq!(state["thinking"], "medium", "{state}");
    assert_eq!(
        state["writes"],
        json!([["mistral-large-4-0", "medium"]]),
        "one session-only write, never the Settings default: {state}"
    );
    let send_alerts = alerts(&mut events).await;
    assert!(send_alerts.is_empty(), "no notice at Send: {send_alerts:?}");
    let resaved = run.saved();
    assert_eq!(resaved, saved, "the record is rewritten once");
    run.finish().await;
}

/// The Settings default is the catalog Large 4 itself (high) and the chat's own level is high too. Vibe already runs the same model, but its warning and Core's "could not restore" notice must not appear.
#[tokio::test]
async fn a_retired_large_4_pin_on_the_same_default_model_is_silent_and_rewritten() {
    let run = Box::pin(start("same-default", &saved_large_4("high"), "large-4")).await;
    // Vibe resumed on the default, which is the same Large 4 here.
    let mut state = run.state();
    state["active"] = json!("mistral-large-4-0");
    state["default"] = json!("mistral-large-4-0");
    std::fs::write(run.home.join("state.json"), state.to_string()).unwrap();
    let shared = run.core.shared.clone();
    let mut events = run.core.subscribe(Some(CHAT.into()));
    let pin = super::recorded(&json!({"model":"large-4","reasoningEffort":"high"}));

    let last = run.resumed().await;
    let reopened = super::restore(&shared, CHAT, last, pin).await;
    assert_eq!(active(&reopened), large_4("high"), "{reopened}");
    assert_eq!(
        reopened["config"]["validationWarnings"],
        json!([]),
        "{reopened}"
    );
    let seen = alerts(&mut events).await;
    assert!(seen.is_empty(), "{seen:?}");
    assert_eq!(run.saved()["remembered"]["alias"], "mistral-large-4-0");
    run.finish().await;
}

/// Control: a saved model that is really gone keeps the existing notice and Settings fallback.
#[tokio::test]
async fn a_really_removed_model_still_takes_the_settings_fallback_with_its_notice() {
    let saved =
        json!({"remembered":{"alias":"retired-flash","effort":"high","display_name":"Old Flash"}});
    let run = Box::pin(start("removed", &saved, "retired-flash")).await;
    let shared = run.core.shared.clone();
    let mut events = run.core.subscribe(Some(CHAT.into()));
    let pin = super::recorded(&json!({"model":"retired-flash","reasoningEffort":"high"}));

    let last = run.resumed().await;
    let reopened = super::restore(&shared, CHAT, last, pin).await;
    assert_eq!(
        active(&reopened),
        ("mistral-medium-latest".into(), "high".into()),
        "{reopened}"
    );
    let seen = alerts(&mut events).await;
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert!(
        seen[0].starts_with("Old Flash with high thinking"),
        "{seen:?}"
    );
    assert_eq!(run.saved()["remembered"]["alias"], "mistral-medium-latest");
    run.finish().await;
}

/// A Send in a chat whose saved choice still names the retired alias (no reopen in between) runs it as Large 4: the
/// record is rewritten and nothing falls back to the Settings default.
#[tokio::test]
async fn a_send_with_a_saved_retired_choice_rewrites_it_without_a_notice() {
    let run = Box::pin(start("send-only", &saved_large_4("high"), "large-4")).await;
    let mut state = run.state();
    state["active"] = json!("mistral-large-4-0");
    state["warnings"] = json!([]);
    std::fs::write(run.home.join("state.json"), state.to_string()).unwrap();
    let shared = run.core.shared.clone();
    let mut events = run.core.subscribe(Some(CHAT.into()));

    super::before(&shared, "turn/start", &json!({"sessionId":CHAT})).await;
    let seen = alerts(&mut events).await;
    assert!(seen.is_empty(), "{seen:?}");
    let state = run.state();
    assert_eq!(state["active"], "mistral-large-4-0", "{state}");
    assert_eq!(state["writes"], json!([]), "{state}");
    assert_eq!(run.saved()["remembered"]["alias"], "mistral-large-4-0");
    run.finish().await;
}

/// A project can use the retired alias and wire name for its own provider. The lobby does not read the project's
/// model row; the chat's own list does. The saved identity must remain intact until that list is available.
/// PREDICT: the launch check rewrites the saved choice to the catalog row before the project list is read.
#[tokio::test]
async fn project_provider_same_wire_name_must_not_migrate_at_lobby_launch() {
    let private = json!({"remembered":{"alias":"large-4","effort":"medium",
        "display_name":"My Private Large 4","name":"mistral-large-4"}});
    let run = Box::pin(start("same-wire-name", &private, "large-4")).await;
    std::fs::write(
        run.home.join("config.toml"),
        "active_model = \"mistral-medium-latest\"\nexperimental_enable_model_catalog = true\n",
    )
    .unwrap();

    let checked = run
        .core
        .execute(crate::Origin::Ui, crate::Command::CheckDefaultModel {})
        .await
        .unwrap();
    assert!(matches!(checked, crate::CommandResult::Reply { .. }));
    assert_eq!(
        run.saved(),
        private,
        "a lobby read cannot identify this chat's provider"
    );

    let project_dir = run.root.join("project");
    std::fs::create_dir_all(&project_dir).unwrap();
    let mut project_models = models();
    project_models.as_array_mut().unwrap().push(json!({
        "alias":"large-4","name":"mistral-large-4","source":"private-provider",
        "displayName":"My Private Large 4","thinking":"medium",
        "thinkingLevels":["off","medium","high"]
    }));
    std::fs::write(
        run.home.join("project-models.json"),
        project_models.to_string(),
    )
    .unwrap();
    let mut state = run.state();
    state["project"] = json!(true);
    state["active"] = json!("large-4");
    state["thinking"] = json!("medium");
    state["warnings"] = json!([]);
    std::fs::write(run.home.join("state.json"), state.to_string()).unwrap();

    let detached = crate::detached_runtime::config(
        &run.core.shared,
        &json!({"session":{"id":CHAT,"cwd":project_dir,"model":"large-4",
            "reasoningEffort":"medium"}}),
    )
    .await
    .unwrap();
    assert_eq!(detached["activeModel"]["displayName"], "My Private Large 4");
    let shared = run.core.shared.clone();
    let mut events = run.core.subscribe(Some(CHAT.into()));
    let pin = super::recorded(&json!({"model":"large-4","reasoningEffort":"medium"}));
    let last = run.resumed().await;
    let reopened = super::restore(&shared, CHAT, last, pin).await;
    assert_eq!(active(&reopened), ("large-4".into(), "medium".into()));
    assert_eq!(run.state()["writes"], json!([]));
    assert_eq!(run.saved(), private);
    let seen = alerts(&mut events).await;
    assert!(seen.is_empty(), "{seen:?}");
    run.finish().await;
}

/// The launch check leaves all saved chat choices alone until each chat's own model list can settle them. This also
/// keeps damaged entries and other chats untouched; no chat's model is written.
/// PREDICT (written before the correction): launch moves the seed and pending records to the catalog Large 4.
#[tokio::test]
async fn the_launch_check_leaves_saved_choices_until_each_chat_opens() {
    let run = Box::pin(start("launch", &saved_large_4("high"), "large-4")).await;
    std::fs::write(
        run.home.join("config.toml"),
        "active_model = \"mistral-large-4-0\"\nexperimental_enable_model_catalog = true\n",
    )
    .unwrap();
    let mut state = run.state();
    state["default"] = json!("mistral-large-4-0");
    std::fs::write(run.home.join("state.json"), state.to_string()).unwrap();
    let medium = json!({"remembered":{"alias":"mistral-medium-latest","effort":"high",
        "display_name":"Mistral Medium 3.5","name":"mistral-medium-latest"}});
    // The app records the model name of every accepted pick (`accepted`), but the lobby cannot settle even that
    // name because a project provider can use the same wire name.
    let pending = json!({
        "pending":{"alias":"large-4","effort":"off","name":"mistral-large-4"},
        "remembered":{"alias":"large-4","effort":"off","display_name":"Mistral Large 4",
            "name":"mistral-large-4"}});
    let damaged = json!({"remembered":{"alias":"large-4","effort":42}});
    std::fs::write(
        run.sessions.join(super::records::FILE),
        json!({"version":1,"chats":{CHAT: saved_large_4("high"), "chat-medium": medium,
            "chat-pending": pending, "chat-damaged": damaged}})
        .to_string(),
    )
    .unwrap();

    let result = run
        .core
        .execute(crate::Origin::Ui, crate::Command::CheckDefaultModel {})
        .await
        .unwrap();
    let crate::CommandResult::Reply { result, .. } = result else {
        panic!("launch check must reply")
    };
    assert!(result["gone"].is_null(), "{result}");

    let file: Value =
        serde_json::from_slice(&std::fs::read(run.sessions.join(super::records::FILE)).unwrap())
            .unwrap();
    let chats = &file["chats"];
    assert_eq!(chats[CHAT], saved_large_4("high"), "{file}");
    assert_eq!(chats["chat-pending"], pending, "{file}");
    assert_eq!(chats["chat-medium"], medium, "{file}");
    assert_eq!(chats["chat-damaged"], damaged, "{file}");
    let detached = crate::detached_runtime::config(
        &run.core.shared,
        &json!({"session":{"id":CHAT,"model":"large-4","reasoningEffort":"high"}}),
    )
    .await
    .unwrap();
    assert_eq!(detached["activeModel"]["alias"], "mistral-large-4-0");
    assert_eq!(detached["activeModel"]["thinking"], "high");
    assert_eq!(detached["activeModel"]["displayName"], "Mistral Large 4");
    assert_eq!(
        run.saved(),
        saved_large_4("high"),
        "view reads do not rewrite records"
    );
    assert_eq!(
        run.state()["writes"],
        json!([]),
        "no chat's model is written"
    );
    run.finish().await;
}

/// Control: while Vibe has not loaded Mistral's catalog (no catalog Large 4 yet) the launch check leaves every saved
/// choice alone, so a later check (or reopen) can still settle it.
#[tokio::test]
async fn the_launch_check_without_the_catalog_leaves_saved_choices_alone() {
    let run = Box::pin(start("launch-offline", &saved_large_4("high"), "large-4")).await;
    std::fs::write(
        run.home.join("config.toml"),
        "active_model = \"mistral-medium-latest\"\nexperimental_enable_model_catalog = true\n",
    )
    .unwrap();
    let local: Vec<Value> = models()
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            let mut row = row.clone();
            row["source"] = json!("local");
            row
        })
        .collect();
    std::fs::write(run.home.join("models.json"), json!(local).to_string()).unwrap();
    let before = std::fs::read(run.sessions.join(super::records::FILE)).unwrap();
    let result = run
        .core
        .execute(crate::Origin::Ui, crate::Command::CheckDefaultModel {})
        .await
        .unwrap();
    assert!(matches!(result, crate::CommandResult::Reply { .. }));
    assert_eq!(
        std::fs::read(run.sessions.join(super::records::FILE)).unwrap(),
        before
    );
    run.finish().await;
}

/// The launch check has no chat folder, so neither the old seed nor a project's own `large-4` nor an unnamed pin is
/// settled there. A chat list read later decides each one.
/// PREDICT (written before the correction): the launch check moves the named old seed alone.
#[tokio::test]
async fn the_launch_check_leaves_seed_project_and_unnamed_alias_choices_alone() {
    let run = Box::pin(start("launch-private", &saved_large_4("high"), "large-4")).await;
    std::fs::write(
        run.home.join("config.toml"),
        "active_model = \"mistral-large-4-0\"\nexperimental_enable_model_catalog = true\n",
    )
    .unwrap();
    let mut state = run.state();
    state["default"] = json!("mistral-large-4-0");
    std::fs::write(run.home.join("state.json"), state.to_string()).unwrap();
    let private = json!({"remembered":{"alias":"large-4","effort":"high",
        "display_name":"My Local Model","name":"private-local-model"}});
    let alias_only = json!({"remembered":{"alias":"large-4","effort":"high"}});
    let all_slots = |mut saved: Value| {
        saved["pending"] = saved["remembered"].clone();
        saved["fallback"] = saved["remembered"].clone();
        saved
    };
    let private = all_slots(private);
    let alias_only = all_slots(alias_only);
    let seed = all_slots(saved_large_4("high"));
    std::fs::write(
        run.sessions.join(super::records::FILE),
        json!({"version":1,"chats":{CHAT: seed, "chat-private": private,
            "chat-unnamed": alias_only}})
        .to_string(),
    )
    .unwrap();

    let result = run
        .core
        .execute(crate::Origin::Ui, crate::Command::CheckDefaultModel {})
        .await
        .unwrap();
    assert!(matches!(result, crate::CommandResult::Reply { .. }));
    let file: Value =
        serde_json::from_slice(&std::fs::read(run.sessions.join(super::records::FILE)).unwrap())
            .unwrap();
    let chats = &file["chats"];
    assert_eq!(chats[CHAT], seed, "{file}");
    assert_eq!(chats["chat-private"], private, "{file}");
    assert_eq!(chats["chat-unnamed"], alias_only, "{file}");
    run.finish().await;
}

/// Reopening the project chat where its own `large-4` row is no longer offered (the project's settings are not
/// loaded): Vibe's pin names only the alias, but the saved choice names the private model, so the chat is not
/// silently moved to the catalog Large 4; it takes the usual Settings fallback with its notice.
///
/// PREDICT (written before the change): Vibe's bare `large-4` pin resolves to the catalog Large 4 and is applied
/// without a notice (`writes == [["mistral-large-4-0","high"]]`, no alert).
#[tokio::test]
async fn reopening_a_project_large_4_chat_never_silently_switches_it_to_catalog_large_4() {
    let private = json!({"remembered":{"alias":"large-4","effort":"high",
        "display_name":"My Local Model","name":"private-local-model"}});
    let run = Box::pin(start("reopen-private", &private, "large-4")).await;
    let shared = run.core.shared.clone();
    let mut events = run.core.subscribe(Some(CHAT.into()));
    let pin = super::recorded(&json!({"model":"large-4","reasoningEffort":"high"}));

    let last = run.resumed().await;
    let reopened = super::restore(&shared, CHAT, last, pin).await;
    assert_ne!(active(&reopened).0, "mistral-large-4-0", "{reopened}");
    let state = run.state();
    assert!(
        !state["writes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|write| write[0] == "mistral-large-4-0"),
        "{state}"
    );
    let seen = alerts(&mut events).await;
    assert_eq!(seen.len(), 1, "the person is told: {seen:?}");
    assert!(seen[0].starts_with("My Local Model"), "{seen:?}");
    run.finish().await;
}

/// A stored chat's display uses the choice that reopening restores: fallback, then pending, then remembered.
/// PREDICT: reading only remembered gives the old seed identity when a private choice is pending or a fallback.
#[tokio::test]
async fn a_detached_choice_uses_the_same_saved_identity_as_reopen() {
    let private = json!({"alias":"large-4","effort":"high",
        "display_name":"My Local Model","name":"private-local-model"});
    for slot in ["pending", "fallback"] {
        let mut saved = saved_large_4("high");
        saved[slot] = private.clone();
        let run = Box::pin(start(&format!("detached-{slot}"), &saved, "large-4")).await;
        assert_eq!(
            run.core.shared.model_choices.saved_name(CHAT, "large-4"),
            "private-local-model",
            "{slot} is the choice reopening restores"
        );
        let config = crate::detached_runtime::config(
            &run.core.shared,
            &json!({"session":{"id":CHAT,"model":"large-4","reasoningEffort":"high"}}),
        )
        .await
        .unwrap();
        assert_eq!(config["activeModel"]["alias"], "large-4", "{config}");
        run.finish().await;
    }
}
