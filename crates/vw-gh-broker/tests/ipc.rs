//! Local endpoint permissions and wire tests for broker callers that do not use the shim.
//! Malformed or extended requests fail closed; Unix's second-user probe needs privilege to switch UID.

#![allow(clippy::unwrap_used)]

mod support;

use interprocess::local_socket::{Stream, prelude::*};
use std::io::{BufRead, BufReader, Write};
use support::Harness;

#[cfg(unix)]
#[test]
fn unix_endpoint_and_parent_are_user_only_and_reclaimed() {
    use std::{os::unix::fs::PermissionsExt, path::Path};
    let harness = Harness::new();
    let endpoint = Path::new(harness.broker.endpoint()).to_owned();
    assert_eq!(
        std::fs::metadata(&endpoint).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        std::fs::metadata(endpoint.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(harness.call(&["repo", "view", "owner/repo"]).exit_code, 0);
    drop(harness);
    assert!(!endpoint.exists());
}

#[cfg(unix)]
#[test]
#[ignore = "requires privilege to launch a second OS user (UID/GID 65534)"]
fn another_os_user_cannot_connect() {
    use std::{os::unix::process::CommandExt, process::Command};
    let harness = Harness::new();
    let result = Command::new("python3")
        .args(["-c", "import os,socket,sys\nassert os.geteuid() == 65534\ns=socket.socket(socket.AF_UNIX)\ntry:\n s.connect(sys.argv[1])\nexcept PermissionError:\n sys.exit(0)\nsys.exit(1)"])
        .arg(harness.broker.endpoint()).uid(65534).gid(65534).output()
        .expect("the second-user probe needs root or CAP_SETUID/CAP_SETGID");
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(!harness.cwd.path().join("runs").exists());
}

#[test]
fn malformed_and_environment_override_requests_do_not_run_gh() {
    let harness = Harness::new();
    for frame in [
        "not json\n",
        "{\"argv\":[\"api\",\"user\"],\"cwd\":\"/\",\"env\":{\"GH_TOKEN\":\"other\"}}\n",
        "{\"argv\":[\"api\",\"user\"],\"cwd\":\"/\",\"executable\":\"evil\"}\n",
    ] {
        std::thread::scope(|scope| {
            let server = scope.spawn(|| harness.broker.serve_one().unwrap());
            let mut stream = connect(harness.broker.endpoint());
            stream.write_all(frame.as_bytes()).unwrap();
            stream.flush().unwrap();
            let mut line = String::new();
            BufReader::new(stream).read_line(&mut line).unwrap();
            let reply: vw_gh_broker::Reply = serde_json::from_str(&line).unwrap();
            server.join().unwrap();
            assert_eq!(reply.exit_code, 1);
            assert_eq!(
                reply.stderr,
                b"gh broker: invalid request; expected argv and cwd\n"
            );
        });
    }
    assert!(!harness.cwd.path().join("runs").exists());
    assert_eq!(harness.call(&["pr", "list"]).exit_code, 0);
}

#[test]
fn shim_sends_only_argv_and_cwd_on_the_wire() {
    use interprocess::local_socket::ListenerOptions;
    use std::process::Command;
    let directory = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    let endpoint = directory
        .path()
        .join("shim.sock")
        .to_str()
        .unwrap()
        .to_owned();
    #[cfg(windows)]
    let endpoint = format!(
        "shim-test-{}",
        directory.path().file_name().unwrap().to_string_lossy()
    );
    let listener = ListenerOptions::new()
        .name(name(&endpoint))
        .create_sync()
        .unwrap();
    std::thread::scope(|scope| {
        let server = scope.spawn(|| {
            let mut stream = BufReader::new(listener.accept().unwrap());
            let mut line = String::new();
            stream.read_line(&mut line).unwrap();
            let request: serde_json::Value = serde_json::from_str(&line).unwrap();
            assert_eq!(request.as_object().unwrap().len(), 2);
            assert_eq!(request["argv"], serde_json::json!(["pr", "view", "17"]));
            assert_eq!(request["cwd"], serde_json::json!(directory.path()));
            assert!(!line.contains(support::TOKEN));
            stream
                .get_mut()
                .write_all(b"{\"exit_code\":8,\"stdout\":[111,107],\"stderr\":[]}\n")
                .unwrap();
            stream.get_mut().flush().unwrap();
        });
        let output = Command::new(env!("CARGO_BIN_EXE_gh"))
            .env_clear()
            .env(vw_gh_broker::ENDPOINT_ENV, &endpoint)
            .current_dir(directory.path())
            .args(["pr", "view", "17"])
            .output()
            .unwrap();
        server.join().unwrap();
        assert_eq!(output.status.code(), Some(8));
        assert_eq!(output.stdout, b"ok");
    });
}

fn connect(endpoint: &str) -> Stream {
    Stream::connect(name(endpoint)).unwrap()
}

fn name(endpoint: &str) -> interprocess::local_socket::Name<'_> {
    #[cfg(unix)]
    {
        endpoint
            .to_fs_name::<interprocess::local_socket::GenericFilePath>()
            .unwrap()
    }
    #[cfg(windows)]
    {
        endpoint
            .to_ns_name::<interprocess::local_socket::GenericNamespaced>()
            .unwrap()
    }
}
