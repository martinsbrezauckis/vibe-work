//! Launcher guards run only on the Vibe releases they were checked against: one shared list
//! (`vibe_guard_versions.py`) that names the version in `vibe.pin`, and a Vibe outside it stops the start with a
//! message instead of running with a guard missing.
//!
//! PREDICT (written before the change): on the earlier tree every test here fails. There is no `vibe_guard_versions.py`
//! (compile error: `include_str!` of a missing file), and each guard returns early on its own version check
//! (`!= "3.1.2": return`, `BUNDLED_GUARD_VIBE_VERSIONS`, `AGENT_WORKTREE_VIBE_VERSIONS`, the launcher's
//! `{"3.0.4", "3.1.0", "3.1.1", "3.1.2"}`), so an unlisted Vibe ran with the guards silently off; there is no
//! `GUARD_REFUSAL` / `start_failure`.

use std::time::Duration;

const VERSIONS: &str = include_str!("../src/vibe_guard_versions.py");
const PIN: &str = include_str!("../../../tools/public/codegen/vibe.pin");

/// Every file that patches Vibe and therefore must run only on a guarded release, with the guards it installs.
const GUARDS: [(&str, &str, usize); 9] = [
    (
        "vibe_tls_guard.py",
        include_str!("../src/vibe_tls_guard.py"),
        1,
    ),
    (
        "vibe_child_stop.py",
        include_str!("../src/vibe_child_stop.py"),
        2,
    ),
    (
        "vibe_plugin_enablement.py",
        include_str!("../src/vibe_plugin_enablement.py"),
        1,
    ),
    (
        "vibe_mcp_projection.py",
        include_str!("../src/vibe_mcp_projection.py"),
        1,
    ),
    (
        "vibe_skill_cleanup.py",
        include_str!("../src/vibe_skill_cleanup.py"),
        1,
    ),
    (
        "vibe_resume_guard.py",
        include_str!("../src/vibe_resume_guard.py"),
        2,
    ),
    (
        "vibe_keychain_guard.py",
        include_str!("../src/vibe_keychain_guard.py"),
        2,
    ),
    (
        "vibe_remote_cache_guard.py",
        include_str!("../src/vibe_remote_cache_guard.py"),
        1,
    ),
    (
        "secret_shell_path.py",
        include_str!("../../vw-core-api/src/agent_tools/secret_shell_path.py"),
        1,
    ),
];

#[test]
fn the_shared_list_names_the_pinned_version() {
    let line = VERSIONS
        .lines()
        .find(|line| line.starts_with("GUARDED_VIBE_VERSIONS = frozenset({"))
        .expect("one list of the Vibe versions the guards were checked against");
    let pinned = format!("\"{}\"", PIN.trim());
    assert!(
        line.contains(&pinned),
        "the pinned Vibe {pinned} is not in the guarded versions: {line}"
    );
}

#[test]
fn no_guard_keeps_its_own_version_check_and_each_calls_the_shared_one() {
    for (name, source, calls) in GUARDS {
        assert!(
            !source.contains("importlib.metadata.version(\"mistral-vibe\")"),
            "{name} reads the Vibe version itself (an own check can return without its guard)"
        );
        assert!(
            !source.contains("VIBE_VERSIONS = frozenset"),
            "{name} keeps its own version list"
        );
        assert_eq!(
            source.matches("require_guarded_vibe(\"").count(),
            calls,
            "{name}: every guard it installs calls the shared check"
        );
    }
    assert!(VERSIONS.contains("raise SystemExit(f\"{GUARD_REFUSAL}{version}"));
}

#[test]
fn every_program_that_runs_a_guard_carries_the_list_first() {
    assert!(vw_supervisor::CHILD_STOP_ADAPTER.starts_with(VERSIONS));
    let market = include_str!("../../vw-core-api/src/market_discovery.rs");
    let secret = include_str!("../../vw-core-api/src/agent_tools/secret_env.rs");
    for (name, rust, guard) in [
        ("market_discovery.rs", market, "vibe_plugin_enablement.py"),
        ("secret_env.rs", secret, "secret_shell_path.py"),
    ] {
        let list = rust
            .find("vibe_guard_versions.py\")")
            .unwrap_or_else(|| panic!("{name} does not prepend the guarded versions"));
        let script = rust.find(&format!("{guard}\")")).unwrap();
        assert!(list < script, "{name}: the list must come before {guard}");
    }
}

#[test]
fn the_refusal_reaches_the_start_failure_message() {
    let python = VERSIONS
        .lines()
        .find_map(|line| line.strip_prefix("GUARD_REFUSAL = \""))
        .and_then(|rest| rest.strip_suffix('"'))
        .unwrap();
    assert_eq!(python, vw_supervisor::GUARD_REFUSAL);
    let refusal = format!(
        "{}9.9.9 yet: it works with Vibe {}. Wait for a Vibe Work update, or use the Vibe version Vibe Work installs.",
        vw_supervisor::GUARD_REFUSAL,
        PIN.trim()
    );
    let handshake = |tail: Vec<String>| vw_supervisor::SupervisorError::Handshake {
        source: Box::new(vw_rpc::RpcError::Timeout {
            method: "initialize".into(),
            id: 1,
            after: Duration::from_secs(1),
        }),
        exit: None,
        stderr_tail: tail,
    };
    let refused = handshake(vec!["noise".into(), format!("{refusal}\r")]);
    assert_eq!(vw_supervisor::start_failure(&refused), refusal);
    let other = handshake(vec!["Traceback (most recent call last):".into()]);
    assert_eq!(
        vw_supervisor::start_failure(&other),
        "initialize handshake failed: initialize (id 1) timed out after 1s"
    );
    // The setup screen's reason: the refusal is a whole sentence on its own; any other failure is prefixed.
    assert_eq!(vw_supervisor::start_failure_message(&refused), refusal);
    assert_eq!(
        vw_supervisor::start_failure_message(&other),
        "Vibe did not start: initialize handshake failed: initialize (id 1) timed out after 1s"
    );
}

// PREDICT (before the update offer used the list): `guarded_vibe_versions` does not exist (compile error), so the
// signed supported-Vibe manifest could offer a Vibe the launcher then refuses to start.
#[test]
fn the_parsed_list_is_the_files_line() {
    let line = VERSIONS
        .lines()
        .find(|line| line.starts_with("GUARDED_VIBE_VERSIONS = frozenset({"))
        .unwrap();
    // Every quoted item of the line, read independently of the crate's parser.
    let mut expected: Vec<String> = line
        .split('"')
        .skip(1)
        .step_by(2)
        .map(str::to_owned)
        .collect();
    expected.sort();
    let mut parsed = vw_supervisor::guarded_vibe_versions();
    parsed.sort();
    assert_eq!(parsed, expected, "{line}");
    assert!(parsed.contains(&PIN.trim().to_owned()));
}

#[test]
fn the_update_check_is_given_the_guarded_list() {
    assert!(
        include_str!("../../../app/src/updater.rs")
            .contains("guarded: vw_vibe_install::guarded_vibe_versions()"),
        "the app builds the update check without the guarded Vibe versions"
    );
    // The end-to-end extension of the list (`vw_vibe_install::e2e_guard`, debug test hosts) is never the app's.
    let updater = include_str!("../../../app/src/updater.rs");
    assert!(
        !updater.contains("e2e_guard") && !updater.contains("VW_E2E_EXTRA_GUARDED_VIBE"),
        "the app's update check uses the test-only guard list extension"
    );
}
