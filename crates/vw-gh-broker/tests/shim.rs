//! Real credential-free shim tests, with fake gh and a fabricated stdin token.
//! No token is supplied to the shim. Its raw request contains only argv/cwd; replies preserve gh behavior.

mod support;

use std::process::Command;
use support::{Harness, TOKEN};
use vw_gh_broker::ENDPOINT_ENV;

#[test]
fn missing_local_gh_names_the_next_step() {
    let output = Command::new(env!("CARGO_BIN_EXE_gh"))
        .env_clear()
        .arg("--version")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let message = String::from_utf8_lossy(&output.stderr);
    assert!(message.contains("install GitHub CLI"), "{message}");
    assert!(message.contains("restart Vibe Work"), "{message}");
}

#[test]
fn shim_has_no_token_and_preserves_cwd_output_and_exit_code() {
    let harness = Harness::new();
    std::fs::write(harness.cwd.path().join("exit-code"), "8").unwrap();
    let argv = ["pr", "checks", "17", "--json", "name,state"];
    std::thread::scope(|scope| {
        let server = scope.spawn(|| harness.broker.serve_one().unwrap());
        let output = Command::new(env!("CARGO_BIN_EXE_gh"))
            .env_clear()
            .env(ENDPOINT_ENV, harness.broker.endpoint())
            .args(argv)
            .current_dir(harness.cwd.path())
            .output()
            .unwrap();
        server.join().unwrap();
        assert_eq!(output.status.code(), Some(8));
        assert!(String::from_utf8_lossy(&output.stdout).contains("gh_token_set=true"));
        assert!(String::from_utf8_lossy(&output.stdout).contains(&format!("argv={argv:?}")));
        assert!(!String::from_utf8_lossy(&output.stdout).contains(TOKEN));
        assert_eq!(output.stderr, b"stderr-token=[REDACTED]\n");
        assert!(output.stdout.ends_with(b"binary=\xff\x00\n"));
    });
}

#[test]
fn shim_write_is_refused_by_broker() {
    let harness = Harness::new();
    std::thread::scope(|scope| {
        let server = scope.spawn(|| harness.broker.serve_one().unwrap());
        let output = Command::new(env!("CARGO_BIN_EXE_gh"))
            .env_clear()
            .env(ENDPOINT_ENV, harness.broker.endpoint())
            .args(["pr", "merge", "17"])
            .current_dir(harness.cwd.path())
            .output()
            .unwrap();
        server.join().unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&output.stderr).contains("read-only"));
        assert!(!harness.cwd.path().join("runs").exists());
    });
}

#[test]
fn shim_fails_plainly_when_endpoint_is_missing() {
    let output = Command::new(env!("CARGO_BIN_EXE_gh"))
        .env_clear()
        .args(["pr", "view"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        output.stderr,
        b"gh broker: GitHub reader is not available (no broker endpoint configured)\n"
    );
}
