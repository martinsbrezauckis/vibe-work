//! The start check of a remembered install: a late answer is a version, not a silence, and a real silence is reported
//! with what happened and the person's next step, never as a version claim.
//!
//! PREDICT (before the change): the start check used the 25 s search-probe limit, so a remembered Vibe whose first
//! cold start takes longer was reported as "This install did not answer; Vibe Work needs Vibe <pinned>." These tests
//! do not compile before (no `probe_within`, `SAVED_PROBE_LIMIT` or `no_answer_warning`); after, a slow fake server
//! is read within a longer limit and the words name the limit and the next step.
use super::*;

#[cfg(unix)]
#[tokio::test]
async fn a_version_matching_server_without_an_interpreter_is_not_selectable() {
    use std::os::unix::fs::PermissionsExt;

    let dir = std::env::temp_dir().join(format!("vw-existing-unguarded-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let program = dir.join("vibe-app-server");
    let marker = dir.join("executed");
    std::fs::write(
        &program,
        format!("#!/bin/sh\n: > '{}'\nread request\nprintf '%s\\n' '{{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{{\"serverInfo\":{{\"name\":\"vibe-app-server\",\"version\":\"3.1.5\"}}}}}}'\n", marker.display()),
    )
    .unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
    let found = find(
        &Discovery {
            path_var: Some(dir.as_os_str().to_owned()),
            ..Discovery::default()
        },
        &dir.join("probes"),
        &[],
        "3.1.5",
        &dir.join("own"),
    )
    .await;
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].version, None);
    assert!(!marker.exists(), "discovery executed an unguardable server");
    assert!(
        !found[0].supported,
        "version alone offered an unguarded server"
    );
    assert_eq!(
        found[0].warning.as_deref(),
        Some(
            "This Vibe installation cannot run with Vibe Work's safety guards: choose Install Vibe for Vibe Work, or a Vibe installed in its own Python environment."
        )
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn a_guardable_candidate_is_still_version_probed() {
    use std::os::unix::fs::PermissionsExt;

    let python = std::env::var("VIBE_PYTHON").expect("the pinned Vibe Python is required");
    let dir = std::env::temp_dir().join(format!("vw-existing-guarded-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let program = dir.join("vibe-app-server");
    let marker = dir.join("executed");
    let reply =
        serde_json::json!({"jsonrpc":"2.0", "id":1, "result":{"serverInfo":{"version":"3.1.5"}}});
    std::fs::write(
        &program,
        format!(
            "#!{python}\nimport pathlib, sys\nsys.stdin.readline()\npathlib.Path({}).write_text('yes')\nprint({})\n",
            serde_json::to_string(&marker.display().to_string()).unwrap(),
            serde_json::to_string(&reply.to_string()).unwrap(),
        ),
    )
    .unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
    let found = find(
        &Discovery {
            path_var: Some(dir.as_os_str().to_owned()),
            ..Discovery::default()
        },
        &dir.join("probes"),
        &[],
        "3.1.5",
        &dir.join("own"),
    )
    .await;
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].version.as_deref(), Some("3.1.5"));
    assert!(found[0].supported);
    assert_eq!(std::fs::read_to_string(&marker).unwrap(), "yes");
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn an_older_initialize_names_the_version_but_never_becomes_supported() {
    use std::os::unix::fs::PermissionsExt;
    let python = std::env::var("VIBE_PYTHON").expect("the pinned Vibe Python is required");
    let dir = std::env::temp_dir().join(format!("vw-probe-old-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let program = dir.join("vibe-app-server");
    for code in ["\"invalid_params\"", "-32602"] {
        let code = serde_json::from_str::<serde_json::Value>(code).unwrap();
        let invalid = serde_json::json!({"jsonrpc":"2.0", "id":1, "error":{"code":code,"message":"Invalid request parameters"}}).to_string();
        let old = serde_json::json!({"jsonrpc":"2.0", "id":2, "result":{"serverInfo":{"version":"2.26.1"}}}).to_string();
        std::fs::write(&program, format!(
            "#!{python}\nimport sys\nfirst = sys.stdin.readline()\nif 'agentWorktrees' not in first: sys.exit(2)\nprint({}, flush=True)\nsecond = sys.stdin.readline()\nif any(key in second for key in ('agentWorktrees', 'sessionPeerRestore', 'entrypoint', 'title')): sys.exit(3)\nprint({}, flush=True)\n",
            serde_json::to_string(&invalid).unwrap(),
            serde_json::to_string(&old).unwrap(),
        )).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
        let found = find(
            &Discovery {
                explicit: None,
                path_var: Some(dir.as_os_str().to_owned()),
                home: None,
            },
            &dir.join("probes"),
            &[],
            "3.1.4",
            &dir.join("own"),
        )
        .await;
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].version.as_deref(), Some("2.26.1"));
        assert!(!found[0].supported);
        assert_eq!(
            found[0].warning.as_deref(),
            Some("This install is Vibe 2.26.1; Vibe Work needs Vibe 3.1.4.")
        );
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn an_initialize_retry_is_bounded_and_missing_versions_stay_refused() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("vw-probe-invalid-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let program = dir.join("vibe-app-server");
    for response in [
        r#"{"result":{}}"#,
        r#"{"result":{"serverInfo":{"version":""}}}"#,
        r#"{"error":{"code":-32602}}"#,
    ] {
        let frame: serde_json::Value = serde_json::from_str(response).unwrap();
        let frame = serde_json::json!({"jsonrpc":"2.0", "id":2, "result":frame.get("result"), "error":frame.get("error")});
        std::fs::write(&program, format!("#!/bin/sh\nread first\nprintf '%s\\n' '{{\"jsonrpc\":\"2.0\",\"id\":1,\"error\":{{\"code\":-32602}}}}'\nread second\nprintf '%s\\n' '{frame}'\nread third\nexit 0\n")).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
        let result = probe_checked(
            Duration::from_secs(3),
            &program,
            &[],
            &dir.join("home"),
            &[],
        )
        .await;
        assert_eq!(result, Err(ProbeFailure::NoVersion), "{response}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn a_late_answer_within_the_limit_is_a_version_not_a_silence() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("vw-probe-late-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let program = dir.join("vibe-app-server");
    std::fs::write(
        &program,
        "#!/bin/sh\nread line\nsleep 2\nprintf '%s\\n' '{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"serverInfo\":{\"name\":\"vibe-app-server\",\"version\":\"3.1.1\"}}}'\n",
    )
    .unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
    let home = dir.join("home");
    let early = probe_within(Duration::from_secs(1), &program, &[], &home, &[]).await;
    let late = probe_within(Duration::from_secs(20), &program, &[], &home, &[]).await;
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(early, None, "a limit shorter than the start is a silence");
    assert_eq!(late.as_deref(), Some("3.1.1"));
}

#[test]
fn the_remembered_install_gets_a_longer_limit_and_honest_words() {
    assert!(SAVED_PROBE_LIMIT > PROBE_LIMIT);
    assert!(SAVED_PROBE_LIMIT >= Duration::from_secs(60));
    let text = no_answer_warning(SAVED_PROBE_LIMIT, "3.1.2");
    assert!(text.contains("did not answer within 60 seconds"), "{text}");
    assert!(text.contains("Start Vibe Work again"), "{text}");
    assert!(text.contains("Vibe Work needs Vibe 3.1.2"), "{text}");
    assert!(!text.contains("This install is Vibe"), "{text}");
}

// PREDICT (before the change): a saved install whose process ends before it answers (a crash on start, a missing
// runtime file) is worded like a silence ("did not answer within 60 seconds"), and the reason it ended (exit code, its
// error text) is thrown away. `probe_checked`, `ProbeFailure` and `saved_warning` do not exist before: these tests do not
// compile. After, an exited program reports its exit code and error text, only a real timeout gets the timeout words, and
// any other failure uses the search's wording for a program that does not answer like Vibe's app server.

#[cfg(unix)]
#[tokio::test]
async fn a_program_that_exits_before_answering_reports_why() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("vw-probe-exit-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let program = dir.join("vibe-app-server");
    std::fs::write(&program, "#!/bin/sh\necho 'no runtime here' >&2\nexit 3\n").unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
    let result = probe_checked(
        Duration::from_secs(20),
        &program,
        &[],
        &dir.join("home"),
        &[],
    )
    .await;
    std::fs::remove_dir_all(&dir).unwrap();
    match result {
        Err(ProbeFailure::Exited { code }) => {
            assert_eq!(code, Some(3));
        }
        other => panic!("expected Exited, got {other:?}"),
    }
}

/// The probe passes the supervisor's environment allowlist (proxy addresses may carry a password); a failing program
/// that prints its environment must not get that text into the failure, which the start check logs.
/// PREDICT (before the change): the failure kept the last 2000 bytes of the program's error output.
#[cfg(unix)]
#[tokio::test]
async fn a_failing_programs_error_output_never_reaches_the_failure() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("vw-probe-stderr-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let program = dir.join("vibe-app-server");
    std::fs::write(
        &program,
        "#!/bin/sh\necho 'HTTPS_PROXY=VW-PROBE-STDERR-SENTINEL' >&2\nexit 4\n",
    )
    .unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
    let result = probe_checked(
        Duration::from_secs(20),
        &program,
        &[],
        &dir.join("home"),
        &[],
    )
    .await;
    std::fs::remove_dir_all(&dir).unwrap();
    let failure = result.expect_err("the program exits before answering");
    assert_eq!(failure, ProbeFailure::Exited { code: Some(4) });
    assert!(
        !format!("{failure:?}").contains("VW-PROBE-STDERR-SENTINEL"),
        "{failure:?}"
    );
}

#[cfg(windows)]
#[tokio::test]
async fn a_program_that_exits_before_answering_reports_why() {
    let dir = std::env::temp_dir().join(format!("vw-probe-exit-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let program = dir.join("vibe-app-server.cmd");
    std::fs::write(&program, "@echo no runtime here 1>&2\r\n@exit /b 3\r\n").unwrap();
    let env: Vec<_> = std::env::vars_os().collect();
    let result = probe_checked(
        Duration::from_secs(20),
        &program,
        &[],
        &dir.join("home"),
        &env,
    )
    .await;
    std::fs::remove_dir_all(&dir).unwrap();
    match result {
        Err(ProbeFailure::Exited { code }) => {
            assert_eq!(code, Some(3));
        }
        other => panic!("expected Exited, got {other:?}"),
    }
}

#[cfg(unix)]
#[tokio::test]
async fn a_silent_program_is_a_timeout() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("vw-probe-quiet-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let program = dir.join("vibe-app-server");
    std::fs::write(&program, "#!/bin/sh\nread line\nsleep 30\n").unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
    let result = probe_checked(
        Duration::from_secs(1),
        &program,
        &[],
        &dir.join("home"),
        &[],
    )
    .await;
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(result, Err(ProbeFailure::TimedOut));
}

#[test]
fn only_a_real_timeout_gets_the_timeout_words() {
    let limit = SAVED_PROBE_LIMIT;
    let timeout = saved_warning(&Err(ProbeFailure::TimedOut), limit, "3.1.2").unwrap();
    assert!(
        timeout.contains("did not answer within 60 seconds"),
        "{timeout}"
    );
    let exited = ProbeFailure::Exited { code: Some(3) };
    for failure in [
        exited,
        ProbeFailure::Spawn("denied".into()),
        ProbeFailure::NoVersion,
    ] {
        let text = saved_warning(&Err(failure), limit, "3.1.2").unwrap();
        assert!(!text.contains("within"), "{text}");
        assert!(
            text.contains("did not answer like Vibe's app server"),
            "{text}"
        );
    }
    let other = saved_warning(&Ok("3.1.1".into()), limit, "3.1.2");
    assert_eq!(other, Some(version_warning("3.1.1", "3.1.2")));
    assert_eq!(saved_warning(&Ok("3.1.2".into()), limit, "3.1.2"), None);
}
