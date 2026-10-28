//! Shared fixture root regressions, using subprocesses to avoid changing the test runner's environment.
//! Depends on the public path helper and std; run by Cargo in both normal and nested checkouts.

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;
use vw_public_tests::test_paths;

#[test]
fn fixture_root_override_and_default_survive_reexecution() {
    if let Some(expected) = std::env::var_os("VW_TEST_EXPECT_ROOT") {
        assert_eq!(
            test_paths::root().canonicalize().unwrap(),
            PathBuf::from(expected).canonicalize().unwrap()
        );
        return;
    }
    let run = test_paths::root().join(format!("root-override-{}", std::process::id()));
    std::fs::create_dir_all(run.join("child")).unwrap();
    let expected = run.canonicalize().unwrap().join("fixtures");
    let override_path = run.join("child/../fixtures");
    assert!(!expected.exists());
    let output = child(&run, &expected)
        .env("VW_TEST_ROOT", &override_path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(expected.is_dir(), "the override folder was not created");

    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .unwrap();
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    repo.hash(&mut hash);
    let default = repo
        .parent()
        .unwrap()
        .join(format!(".vw-tests-{:016x}", hash.finish()));
    std::fs::create_dir_all(&default).unwrap();
    let output = child(&run, &default.canonicalize().unwrap())
        .env_remove("VW_TEST_ROOT")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let output = child(&run, &expected)
        .env("VW_TEST_ROOT", "relative-test-root")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("VW_TEST_ROOT must be an absolute folder")
    );
    std::fs::remove_dir_all(run).unwrap();
}

fn child(run: &Path, expected: &Path) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "fixture_root_override_and_default_survive_reexecution",
            "--nocapture",
        ])
        .env("VW_TEST_EXPECT_ROOT", expected)
        .env("HOME", run.join("home"))
        .env("TMPDIR", run.join("tmp"))
        .env("TEMP", run.join("temp"));
    command
}
