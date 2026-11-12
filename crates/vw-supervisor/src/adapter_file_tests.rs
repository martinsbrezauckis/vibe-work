//! Adapter delivery security and lifecycle checks; runs through the same loader as the supervisor.
//! Uses disposable app homes, vw-platform permissions and the configured Python; no Vibe install is edited.

use super::*;
use std::io::Write;
use std::process::{Command, Stdio};

struct Home(PathBuf);
impl Home {
    fn new(name: &str) -> Self {
        let base = std::env::var_os("VW_TEST_ROOT").map_or_else(std::env::temp_dir, PathBuf::from);
        let path = base.join(format!("vw-adapter-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Home {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn private_adapter_is_complete_verified_and_reused_across_restarts() {
    let home = Home::new("restart");
    let path = prepare(&home.0).unwrap();
    assert_eq!(path, adapter_path(&home.0));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), CHILD_STOP_ADAPTER);
    for private in [&home.0, &home.0.join(".launcher"), &path] {
        assert!(vw_platform::is_user_only(private).unwrap(), "{private:?}");
    }
    let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
    assert_eq!(prepare(&home.0).unwrap(), path);
    assert_eq!(
        std::fs::metadata(&path).unwrap().modified().unwrap(),
        modified
    );
    assert_eq!(
        std::fs::read_dir(home.0.join(".launcher")).unwrap().count(),
        1
    );
}

#[test]
fn tampered_adapter_is_refused_before_spawn_without_overwriting_it() {
    let home = Home::new("tampered");
    let path = prepare(&home.0).unwrap();
    std::fs::write(&path, "raise RuntimeError('planted')\n").unwrap();
    assert!(
        prepare(&home.0)
            .unwrap_err()
            .to_string()
            .contains("integrity check failed")
    );
    assert_eq!(
        std::fs::read_to_string(path).unwrap(),
        "raise RuntimeError('planted')\n"
    );
    assert!(prepare(Path::new("relative-home")).is_err());
}

#[cfg(unix)]
#[test]
fn linked_home_directory_and_file_and_non_private_file_are_refused() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let root = Home::new("links");
    let outside = root.0.join("outside");
    std::fs::create_dir(&outside).unwrap();
    let home = root.0.join("home");
    symlink(&outside, &home).unwrap();
    assert!(prepare(&home).is_err());
    std::fs::remove_file(&home).unwrap();
    std::fs::create_dir(&home).unwrap();
    symlink(&outside, home.join(".launcher")).unwrap();
    assert!(prepare(&home).is_err());
    std::fs::remove_file(home.join(".launcher")).unwrap();
    let path = prepare(&home).unwrap();
    std::fs::remove_file(&path).unwrap();
    let planted = outside.join("planted.py");
    std::fs::write(&planted, CHILD_STOP_ADAPTER).unwrap();
    symlink(&planted, &path).unwrap();
    assert!(prepare(&home).is_err());
    std::fs::remove_file(&path).unwrap();
    let path = prepare(&home).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).unwrap();
    assert!(
        prepare(&home)
            .unwrap_err()
            .to_string()
            .contains("not owner-only")
    );
}

fn loader(path: &Path, digest: &str, cwd: &Path) -> Command {
    let python = std::env::var_os("VIBE_PYTHON").unwrap_or_else(|| {
        if cfg!(windows) {
            "python".into()
        } else {
            "python3".into()
        }
    });
    let mut cmd = Command::new(python);
    cmd.args(["-I", "-u", "-c", ADAPTER_BOOTSTRAP])
        .arg(path)
        .arg(digest)
        .args(["vibe-app-server", "--multiplex"])
        .current_dir(cwd)
        .env("PYTHONPATH", cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd
}

#[test]
fn loader_keeps_stdin_argv_and_isolation_and_refuses_a_post_preparation_swap() {
    let home = Home::new("loader");
    let source = "import sys\nassert sys.flags.isolated == 1\nassert sys.argv[1:] == ['vibe-app-server', '--multiplex']\nprint(sys.stdin.readline().strip())\n";
    let path = home.0.join("adapter.py");
    std::fs::write(&path, source).unwrap();
    // -I must exclude both the working folder and PYTHONPATH before the verifier imports hashlib.
    std::fs::write(
        home.0.join("hashlib.py"),
        "raise RuntimeError('planted import')",
    )
    .unwrap();
    let digest = format!("{:x}", Sha256::digest(source.as_bytes()));
    let mut child = loader(&path, &digest, &home.0).spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"rpc remains available\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "rpc remains available"
    );
    // A file replaced after Rust's check cannot run even a first statement in Python.
    std::fs::write(&path, "print('planted code ran')\n").unwrap();
    let output = loader(&path, &digest, &home.0).output().unwrap();
    assert!(!output.status.success());
    assert_eq!(output.stdout, [] as [u8; 0]);
    assert!(String::from_utf8_lossy(&output.stderr).contains("integrity check failed"));
    assert!(
        ADAPTER_BOOTSTRAP.len() < 1_024,
        "the verifier must remain bounded"
    );
}
