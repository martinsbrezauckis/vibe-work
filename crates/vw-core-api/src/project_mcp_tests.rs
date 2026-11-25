//! Decision tests for project MCP inspection, independent of Vibe's process.

use super::{
    Risk, add_workspace_roots, check_checkout, config_risk, must_read_unknown_session, plugin_risk,
    worktree_target,
};
use serde_json::json;

fn project(text: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".vibe")).unwrap();
    std::fs::write(dir.path().join(".vibe/config.toml"), text).unwrap();
    dir
}

#[test]
fn both_vibe_toml_forms_block_stdio_including_disabled_rows() {
    for config in [
        "[[mcp_servers]]\nname='cmd'\ntransport='stdio'\ncommand='/bin/echo'\n",
        "mcp_servers = [{name='cmd', transport='stdio', command='/bin/echo'}]\n",
    ] {
        let folder = project(config);
        assert_eq!(config_risk(folder.path()), Some(Risk::Command));
    }
    for config in [
        "[[mcp_servers]]\nname='cmd'\ntransport='stdio'\ncommand='/bin/echo'\ndisabled=true\n",
        "mcp_servers = [{name='cmd', transport='stdio', command='/bin/echo', disabled=true}]\n",
    ] {
        let folder = project(config);
        assert_eq!(config_risk(folder.path()), Some(Risk::Command));
    }
}

#[test]
fn unsupported_shapes_fail_closed_without_blocking_clean_or_remote_config() {
    for config in [
        "mcp_servers = 'not an array'\n",
        "mcp_servers = [42]\n",
        "mcp_servers = [{name='unknown'}]\n",
        "[[mcp_servers]]\nname='odd'\ntransport='http'\ncommand='/bin/echo'\n",
        "[[mcp_servers]]\nname='cmd'\ntransport='stdio'\ncommand='/bin/echo'\ndisabled='false'\n",
        "[[mcp_servers]\n",
    ] {
        let folder = project(config);
        assert_eq!(config_risk(folder.path()), Some(Risk::Uncheckable));
    }
    for config in [
        "active_model = 'safe'\n",
        "[[mcp_servers]]\nname='remote'\ntransport='http'\nurl='https://example.invalid/mcp'\n",
    ] {
        let folder = project(config);
        assert_eq!(config_risk(folder.path()), None);
    }
}

#[test]
fn nearest_project_config_wins_and_parent_is_checked_if_nearer_one_is_missing() {
    let folder = project("[[mcp_servers]]\nname='cmd'\ntransport='stdio'\ncommand='/bin/echo'\n");
    let child = folder.path().join("child");
    std::fs::create_dir_all(&child).unwrap();
    assert_eq!(config_risk(&child), Some(Risk::Command));
    std::fs::create_dir_all(child.join(".vibe")).unwrap();
    std::fs::write(child.join(".vibe/config.toml"), "active_model='safe'\n").unwrap();
    assert_eq!(config_risk(&child), None);
}

#[test]
fn existing_worktree_is_inspected_and_future_worktree_moves_are_refused() {
    let root = std::path::Path::new("/safe/project");
    assert_eq!(worktree_target(None, root), Ok(None));
    assert_eq!(
        worktree_target(Some(&serde_json::Value::Null), root),
        Ok(None)
    );
    assert_eq!(
        worktree_target(Some(&json!({"kind":"existing","cwd":"../linked"})), root),
        Ok(Some(std::path::PathBuf::from("/safe/project/../linked")))
    );
    for value in [
        json!({"kind":"auto"}),
        json!({"kind":"existing","cwd":""}),
        json!({"kind":"existing"}),
    ] {
        assert_eq!(worktree_target(Some(&value), root), Err(Risk::WorktreeMove));
    }
    assert_eq!(
        worktree_target(
            Some(&json!({"kind":"create","branch":"a","name":"b"})),
            root
        ),
        Ok(None)
    );
}

#[test]
fn project_plugin_stdio_is_named_and_other_plugin_transports_do_not_block() {
    let root = tempfile::tempdir().unwrap();
    let plugin = root.path().join(".vibe/plugins/probe");
    std::fs::create_dir_all(&plugin).unwrap();
    let file = plugin.join("mcp.json");
    std::fs::write(
        &file,
        r#"{"mcpServers":{"local":{"type":"stdio","command":"sh"}}}"#,
    )
    .unwrap();
    assert_eq!(
        plugin_risk(root.path()),
        Some(Risk::PluginCommand("probe".to_owned(), file.clone()))
    );
    std::fs::write(
        &file,
        r#"{"mcpServers":{"remote":{"type":"http","url":"https://example.invalid"}}}"#,
    )
    .unwrap();
    assert_eq!(plugin_risk(root.path()), None);
    std::fs::write(&file, r#"{"mcpServers":{"local":{"command":"sh"}}}"#).unwrap();
    assert_eq!(plugin_risk(root.path()), Some(Risk::Uncheckable));
}

#[test]
fn extra_workspace_roots_are_project_inputs() {
    let mut roots = std::collections::HashSet::new();
    let default = std::path::Path::new("/safe");
    add_workspace_roots(&mut roots, Some(&json!(["one", "/two"])), default).unwrap();
    assert!(roots.contains(&std::path::PathBuf::from("/safe/one")));
    assert!(roots.contains(&std::path::PathBuf::from("/two")));
    assert_eq!(
        add_workspace_roots(&mut roots, Some(&json!([42])), default),
        Err(Risk::Uncheckable)
    );
}

#[test]
fn an_unlisted_session_folder_is_read_before_plugin_load_or_forced_refresh() {
    let session = json!({"sessionId":"stored"});
    for method in [
        "session/resume",
        "config/read",
        "config/reload",
        "plugin/reload",
        "mcp_catalog/refresh",
        "mcp/refresh",
        "turn/start",
    ] {
        assert!(must_read_unknown_session(method, &session), "{method}");
    }
    assert!(!must_read_unknown_session("config/read", &json!({})));
    assert!(!must_read_unknown_session("session/read", &session));
}

fn checkout_repo() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    vw_changes::prepare_root(&root.path().join("changes")).unwrap();
    git(root.path(), &["init", "-q"]);
    git_commit(root.path(), "empty", true);
    root
}

fn git(root: &std::path::Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .args(args)
        .current_dir(root)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?} failed");
}

fn git_commit(root: &std::path::Path, message: &str, allow_empty: bool) {
    let mut args = vec![
        "-c",
        "user.name=Fixture",
        "-c",
        "user.email=fixture@example.invalid",
        "commit",
        "-qm",
        message,
    ];
    if allow_empty {
        args.push("--allow-empty");
    }
    git(root, &args);
}

fn head(root: &std::path::Path) -> String {
    String::from_utf8(
        std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(root)
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .to_owned()
}

fn commit_config(root: &std::path::Path, text: &str, message: &str) {
    std::fs::create_dir_all(root.join(".vibe")).unwrap();
    std::fs::write(root.join(".vibe/config.toml"), text).unwrap();
    git(root, &["add", ".vibe/config.toml"]);
    git_commit(root, message, false);
}

#[tokio::test]
async fn checkout_preflight_allows_clean_and_http_project_config() {
    let root = checkout_repo();
    assert!(
        check_checkout(root.path().to_path_buf(), head(root.path()))
            .await
            .is_ok()
    );
    commit_config(root.path(), "active_model='safe'\n", "project config");
    assert!(
        check_checkout(root.path().to_path_buf(), head(root.path()))
            .await
            .is_ok()
    );
    commit_config(
        root.path(),
        "[[mcp_servers]]\nname='remote'\ntransport='http'\nurl='https://example.invalid/mcp'\n",
        "remote only",
    );
    assert!(
        check_checkout(root.path().to_path_buf(), head(root.path()))
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn checkout_preflight_refuses_a_committed_project_mcp_file() {
    let root = checkout_repo();
    for (text, message) in [
        (
            "[[mcp_servers]]\nname='cmd'\ntransport='stdio'\ncommand='sh'\n",
            "project command",
        ),
        (
            "mcp_servers=[{name='cmd',transport='stdio',command='sh'}]\n",
            "inline project command",
        ),
    ] {
        commit_config(root.path(), text, message);
        let error = check_checkout(root.path().to_path_buf(), head(root.path()))
            .await
            .unwrap_err();
        assert_eq!(
            error.server_code.as_deref(),
            Some("projectCommandServerBlocked")
        );
    }
    std::fs::write(
        root.path().join(".vibe/config.toml"),
        "enable_telemetry=false\n",
    )
    .unwrap();
    let plugin = root.path().join(".vibe/plugins/probe/mcp.json");
    std::fs::create_dir_all(plugin.parent().unwrap()).unwrap();
    std::fs::write(
        &plugin,
        r#"{"mcpServers":{"local":{"type":"stdio","command":"sh"}}}"#,
    )
    .unwrap();
    git(root.path(), &["add", ".vibe"]);
    git_commit(root.path(), "plugin project command", false);
    let error = check_checkout(root.path().to_path_buf(), head(root.path()))
        .await
        .unwrap_err();
    assert_eq!(
        error.server_code.as_deref(),
        Some("projectPluginCommandServerBlocked")
    );
}
