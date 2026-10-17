//! Real shim and standalone broker binary tests, with fake gh and a fabricated stdin token.
//! No token is supplied to the shim. Its raw request contains only argv/cwd; replies preserve gh behavior.

mod support;

use std::{
    io::{BufRead, BufReader, Write},
    process::{Command, Stdio},
};
use support::{Harness, TOKEN};
use vw_gh_broker::ENDPOINT_ENV;

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
    assert_eq!(output.stderr, b"gh broker: no broker endpoint configured\n");
}

#[test]
fn standalone_broker_reads_token_from_stdin_and_clears_ambient_child_overrides() {
    let cwd = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_vw-gh-broker"))
        .arg("--gh")
        .arg(support::fake_gh())
        .env("GH_DEBUG", "api")
        .env("GITHUB_TOKEN", "fake-ambient-token")
        .env("GH_ENTERPRISE_TOKEN", "fake-enterprise-token")
        .env("GH_REPO", "evil.example/o/r")
        .env("GH_BROWSER", "untrusted-browser")
        .env("HTTPS_PROXY", "http://evil.example")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        writeln!(child.stdin.take().unwrap(), "{TOKEN}").unwrap();
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        assert!(!line.contains(TOKEN));
        let request = vw_gh_broker::Request {
            argv: vec!["api".to_owned(), "user".to_owned()],
            cwd: cwd.path().to_owned(),
        };
        let reply = vw_gh_broker::call(line.trim(), &request).unwrap();
        assert_eq!(reply.exit_code, 0);
        assert!(String::from_utf8_lossy(&reply.stdout).contains("unexpected_env=false"));
        assert!(String::from_utf8_lossy(&reply.stdout).contains("gh_token_set=true"));
        assert!(!String::from_utf8_lossy(&reply.stdout).contains(TOKEN));
    }));
    child.kill().unwrap();
    child.wait().unwrap();
    result.unwrap();
}
