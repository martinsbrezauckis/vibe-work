//! Shared fake-gh compilation and request fixtures for broker integration tests; std and tempfile only.
//! Every invocation stays in a disposable cwd and uses a fabricated token.

#![allow(dead_code, clippy::unwrap_used)]

use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::OnceLock,
};
use tempfile::TempDir;
use vw_gh_broker::{Broker, Reply, Request, call};

pub const TOKEN: &str = "fabricated-gh-broker-token-ONLY-TESTS";

pub fn fake_gh() -> &'static Path {
    static EXECUTABLE: OnceLock<(TempDir, PathBuf)> = OnceLock::new();
    &EXECUTABLE
        .get_or_init(|| {
            let dir = tempfile::tempdir().unwrap();
            let executable = dir
                .path()
                .join(format!("fake-gh{}", std::env::consts::EXE_SUFFIX));
            let result = Command::new("rustc")
                .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/fake_gh.rs"))
                .args(["--edition=2024", "-o"])
                .arg(&executable)
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            (dir, executable)
        })
        .1
}

pub struct Harness {
    pub broker: Broker,
    pub cwd: TempDir,
}

impl Harness {
    pub fn new() -> Self {
        Self {
            broker: Broker::bind(TOKEN.to_owned(), fake_gh()).unwrap(),
            cwd: tempfile::tempdir().unwrap(),
        }
    }

    pub fn request(&self, argv: &[&str]) -> Request {
        Request {
            argv: argv.iter().map(|arg| (*arg).to_owned()).collect(),
            cwd: self.cwd.path().to_owned(),
        }
    }

    pub fn call(&self, argv: &[&str]) -> Reply {
        std::thread::scope(|scope| {
            let server = scope.spawn(|| self.broker.serve_one().unwrap());
            let reply = call(self.broker.endpoint(), &self.request(argv)).unwrap();
            server.join().unwrap();
            reply
        })
    }
}
