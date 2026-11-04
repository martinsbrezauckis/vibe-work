//! Editor file-rule regressions through the panel request/answer boundary, without a window or Vibe server.
//! Uses the core fixture and real policy files; both unsaved text and selections must stay behind the rules.
use super::*;
use crate::feature_test_core::Fixture;
use crate::panel_view::PanelSelection;
use std::path::Path;

fn agent_answer(view: PanelView) -> Value {
    super::agent_answer(view, |_| None)
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn editor(path: &str) -> PanelEditor {
    PanelEditor {
        path: path.into(),
        focused: true,
        side: Some("left".into()),
        saved_version: None,
        unsaved: true,
        unsaved_text: Some("unsaved private key".into()),
        selection: Some(PanelSelection {
            start_line: 2,
            end_line: 3,
            text: "selected private key".into(),
        }),
        cursor_line: Some(3),
        changed_lines: vec![2],
    }
}

fn view(paths: &[&str]) -> PanelView {
    PanelView {
        open: true,
        tabs: vec!["Files".into()],
        active: Some("Files".into()),
        editors: paths.iter().map(|path| editor(path)).collect(),
        files: None,
    }
}

async fn fixture() -> Fixture {
    let f = Fixture::new().await;
    f.load("chat", &f.root.join("project"), &[], false);
    f.core
        .shared
        .access
        .observe_runtime("chat", &json!({"activeAgent":{"name":"accept-edits"}}));
    f
}

async fn read(f: &Fixture, view: PanelView) -> Value {
    read_after(f, view, || {}).await
}

async fn read_after(f: &Fixture, view: PanelView, update: impl FnOnce()) -> Value {
    let shared = &f.core.shared;
    let mut events = shared.events.subscribe();
    let window = async {
        loop {
            if let CoreEvent::PanelRead {
                request_id,
                session_id,
            } = events.recv().await.unwrap()
            {
                assert_eq!(session_id, "chat");
                update();
                assert!(shared.panel_reads.answer(&session_id, &request_id, view));
                break;
            }
        }
    };
    let (result, ()) = tokio::join!(tool(shared, "chat", &Value::Null), window);
    result.unwrap()
}

fn withheld(editor: &Value, path: &str) {
    assert!(editor.get("unsavedText").is_none(), "text leaked: {editor}");
    assert!(
        editor.get("selection").is_none(),
        "selection leaked: {editor}"
    );
    assert_eq!(editor["path"], path);
    assert_eq!(editor["status"], "withheld");
    assert_eq!(
        editor["reason"],
        "Withheld by the person's file read rules. Do not read this file through another tool."
    );
}

#[tokio::test]
async fn denylist_withholds_editor_text_and_selection() {
    let f = fixture().await;
    write(
        &f.root.join("project/.vibe/config.toml"),
        "[tools.read_file]\ndenylist = ['**/*.pem']",
    );
    let outside = f.root.join("outside/server.pem").display().to_string();
    let answer = read(&f, view(&["keys/server.pem", &outside, "src/main.rs", ""])).await;
    withheld(&answer["editors"][0], "keys/server.pem");
    withheld(&answer["editors"][1], &outside);
    let unchanged = agent_answer(view(&["src/main.rs", ""]));
    assert_eq!(answer["editors"][2], unchanged["editors"][0]);
    assert_eq!(answer["editors"][3], unchanged["editors"][1]);
    f.finish().await;
}

#[tokio::test]
async fn sensitive_patterns_withhold_editor_text_and_selection() {
    let f = fixture().await;
    write(
        &f.core.shared.instructions.own_home.join("config.toml"),
        "[tools.read_file]\nsensitive_patterns = ['**/*.pem']",
    );
    let answer = read(&f, view(&["keys/server.pem"])).await;
    withheld(&answer["editors"][0], "keys/server.pem");
    assert_eq!(answer["notes"], json!([]));
    f.finish().await;
}

#[tokio::test]
async fn no_rules_nonmatching_paths_and_untitled_editors_keep_their_output() {
    let f = fixture().await;
    let open = view(&["keys/server.pem", "src/main.rs", ""]);
    assert_eq!(read(&f, open.clone()).await, agent_answer(open));
    write(
        &f.root.join("project/.vibe/config.toml"),
        "[tools.read_file]\ndenylist = ['**/*.pem']",
    );
    let open = view(&["src/main.rs", ""]);
    assert_eq!(read(&f, open.clone()).await, agent_answer(open));
    f.finish().await;
}

#[tokio::test]
async fn disabled_and_never_file_reads_still_withhold_content() {
    let f = fixture().await;
    for policy in [
        "disabled_tools = ['read_file']",
        "[tools.read_file]\npermission = 'never'",
    ] {
        write(&f.root.join("project/.vibe/config.toml"), policy);
        let answer = read(&f, view(&["keys/server.pem", "src/main.rs", ""])).await;
        withheld(&answer["editors"][0], "keys/server.pem");
        withheld(&answer["editors"][1], "src/main.rs");
        assert_eq!(
            answer["editors"][2],
            agent_answer(view(&[""]))["editors"][0]
        );
    }
    f.finish().await;
}

#[tokio::test]
async fn active_mode_and_folder_layers_all_apply() {
    let f = fixture().await;
    let home = &f.core.shared.instructions.own_home;
    let extra = f.root.join("extra");
    std::fs::create_dir_all(&extra).unwrap();
    f.load("chat", &f.root.join("project"), &[extra], false);
    for (path, content) in [
        (
            home.join("agents/accept-edits.toml"),
            "[tools.read_file]\ndenylist = ['**/*.pem']".to_owned(),
        ),
        (
            f.root.join("project/.vibe/agents/accept-edits.toml"),
            "[tools.read_file]\nsensitive_patterns = ['**/*.pem']".into(),
        ),
        (
            f.root.join(".vibe/config.toml"),
            "[tools.read_file]\ndenylist = ['**/*.pem']".into(),
        ),
        (
            f.root.join(".vibe/agents/accept-edits.toml"),
            "[tools.read_file]\nsensitive_patterns = ['**/*.pem']".into(),
        ),
        (
            f.root.join("extra/.vibe/config.toml"),
            "[tools.read_file]\ndenylist = ['**/*.pem']".into(),
        ),
        (
            f.root.join("extra/.vibe/agents/accept-edits.toml"),
            "[tools.read_file]\nsensitive_patterns = ['**/*.pem']".into(),
        ),
        (
            home.join("managed_config_cache.json"),
            json!({"policy":{"payload":"[tools.read_file]\ndenylist = ['**/*.pem']"}}).to_string(),
        ),
    ] {
        write(&path, &content);
        let answer = read(&f, view(&["keys/server.pem"])).await;
        withheld(&answer["editors"][0], "keys/server.pem");
        std::fs::remove_file(path).unwrap();
    }
    f.finish().await;
}

#[tokio::test]
async fn inactive_profiles_do_not_restrict_the_active_mode() {
    let f = fixture().await;
    write(
        &f.root.join("project/.vibe/agents/explore.toml"),
        "[tools.read_file]\ndenylist = ['**/*.pem']",
    );
    let open = view(&["keys/server.pem"]);
    assert_eq!(read(&f, open.clone()).await, agent_answer(open.clone()));
    f.core
        .shared
        .access
        .observe_runtime("chat", &json!({"activeAgent":{"name":"explore"}}));
    withheld(&read(&f, open).await["editors"][0], "keys/server.pem");
    f.finish().await;
}

#[tokio::test]
async fn rules_are_checked_after_the_window_replies() {
    let f = fixture().await;
    let answer = read_after(&f, view(&["keys/server.pem"]), || {
        write(
            &f.root.join("project/.vibe/config.toml"),
            "[tools.read_file]\ndenylist = ['**/*.pem']",
        );
    })
    .await;
    withheld(&answer["editors"][0], "keys/server.pem");
    f.finish().await;
}

#[tokio::test]
async fn unreadable_policy_withholds_paths_but_keeps_untitled_editors() {
    let f = fixture().await;
    let config = f.root.join("project/.vibe/config.toml");
    for invalid in ["broken = [", "[tools.read_file]\ndenylist = [42]"] {
        write(&config, invalid);
        unchecked(&read(&f, view(&["keys/server.pem", ""])).await);
    }
    std::fs::remove_file(&config).unwrap();
    std::fs::create_dir(&config).unwrap();
    unchecked(&read(&f, view(&["keys/server.pem", ""])).await);
    f.finish().await;
}

fn unchecked(answer: &Value) {
    let first = &answer["editors"][0];
    assert_eq!(first["path"], "keys/server.pem");
    assert_eq!(first["status"], "withheld");
    assert!(first.get("unsavedText").is_none());
    assert!(first.get("selection").is_none());
    assert_eq!(
        first["reason"],
        "File read rules could not be checked. Do not read this file through another tool."
    );
    assert_eq!(
        answer["editors"][1],
        agent_answer(view(&[""]))["editors"][0]
    );
}

#[tokio::test]
async fn an_unknown_active_mode_fails_closed() {
    let f = Fixture::new().await;
    f.load("chat", &f.root.join("project"), &[], false);
    unchecked(&read(&f, view(&["keys/server.pem", ""])).await);
    f.finish().await;
}

#[tokio::test]
async fn ask_permission_keeps_the_approved_panel_answer() {
    let f = fixture().await;
    write(
        &f.root.join("project/.vibe/config.toml"),
        "[tools.read_file]\npermission = 'ask'",
    );
    let open = view(&["src/main.rs"]);
    assert_eq!(read(&f, open.clone()).await, agent_answer(open));
    f.finish().await;
}

#[cfg(unix)]
#[tokio::test]
async fn a_path_alias_cannot_bypass_the_file_rules() {
    let f = fixture().await;
    let outside = f.root.join("outside/server.pem");
    write(&outside, "saved private key");
    std::os::unix::fs::symlink(&outside, f.root.join("project/alias.txt")).unwrap();
    write(
        &f.root.join("project/.vibe/config.toml"),
        "[tools.read_file]\ndenylist = ['**/*.pem']",
    );
    withheld(
        &read(&f, view(&["alias.txt"])).await["editors"][0],
        "alias.txt",
    );
    f.finish().await;
}

#[cfg(unix)]
#[tokio::test]
async fn a_linked_policy_file_fails_closed() {
    let f = fixture().await;
    let policy = f.root.join("policy.toml");
    write(&policy, "");
    let config = f.root.join("project/.vibe/config.toml");
    std::fs::create_dir_all(config.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(policy, config).unwrap();
    unchecked(&read(&f, view(&["keys/server.pem", ""])).await);
    f.finish().await;
}
