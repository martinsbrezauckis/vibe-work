//! Browser OS boundary regressions: native identity, packaging, executable checks, query deadline and child reaping.
use super::*;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[test]
fn native_candidates_are_browser_specific() {
    assert_ne!(Browser::Chrome.app_path(), Browser::Edge.app_path());
    assert_ne!(Browser::Chrome.windows_path(), Browser::Edge.windows_path());
}

#[cfg(unix)]
#[test]
fn wrapper_checks_keep_native_chrome_and_reject_container_launchers() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("google-chrome");
    for script in [
        "#!/bin/sh\nexec /snap/bin/chromium \"$@\"",
        "#!/bin/sh\nexec flatpak run com.google.Chrome \"$@\"",
    ] {
        std::fs::write(&path, script).unwrap();
        assert!(discovery::sandboxed(&path));
    }
    std::fs::write(
        &path,
        "#!/bin/sh\n# not a flatpak run\nexec /opt/google/chrome/chrome \"$@\"",
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(!discovery::executable(&path));
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(discovery::executable(&path));
    assert!(!discovery::sandboxed(&path));
    assert!(!discovery::executable(&root.path().join("missing")));
}

#[cfg(unix)]
#[test]
fn a_stalled_query_is_killed_and_reaped_at_its_deadline() {
    let root = tempfile::tempdir().unwrap();
    let pid = root.path().join("pid");
    let mut command = std::process::Command::new("/bin/sh");
    command
        .args(["-c", "echo $$ > \"$1\"; exec /bin/sleep 20", "fixture"])
        .arg(&pid);
    let start = Instant::now();
    let result = process::bounded(&mut command, Duration::from_millis(100));
    assert_eq!(result, (ProcessState::TimedOut, None));
    assert!(start.elapsed() < Duration::from_secs(1));
    #[cfg(target_os = "linux")]
    assert!(
        !PathBuf::from(format!(
            "/proc/{}/stat",
            std::fs::read_to_string(pid).unwrap().trim()
        ))
        .exists()
    );
}

#[cfg(unix)]
#[test]
fn process_results_keep_unknown_separate_from_closed() {
    for (script, expected, ids) in [
        (
            "printf '123\\n'; exit 0",
            ProcessState::Running,
            Some(vec![123]),
        ),
        ("exit 1", ProcessState::Stopped, Some(vec![])),
        ("exit 2", ProcessState::Unknown, None),
    ] {
        let mut command = std::process::Command::new("/bin/sh");
        command.args(["-c", script]);
        assert_eq!(
            process::bounded(&mut command, Duration::from_secs(1)),
            (expected, ids)
        );
    }
    let mut command = std::process::Command::new("/does-not-exist/browser-probe");
    assert_eq!(
        process::bounded(&mut command, Duration::from_secs(1)),
        (ProcessState::Unknown, None)
    );
}

#[cfg(target_os = "linux")]
#[test]
fn isolated_discovery_distinguishes_native_absent_wrapped_and_other_browser() {
    const FIXTURE: &str = "VW_BROWSER_DISCOVERY_FIXTURE";
    if let Some(root) = std::env::var_os(FIXTURE) {
        let root = PathBuf::from(root);
        let home = root.join("home");
        assert_eq!(
            discover(Browser::Chrome, &home),
            Err(InstallationIssue::Missing)
        );
        assert_eq!(
            discover(Browser::Edge, &home),
            Err(InstallationIssue::Missing)
        );
        let chrome = root.join("google-chrome");
        let edge = root.join("microsoft-edge");
        for path in [&chrome, &edge] {
            std::fs::write(path, "#!/bin/sh\nexit 0\n").unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        assert_eq!(discover(Browser::Chrome, &home), Ok(chrome.clone()));
        assert_eq!(discover(Browser::Edge, &home), Ok(edge.clone()));
        std::fs::remove_file(&edge).unwrap();
        assert_eq!(
            discover(Browser::Edge, &home),
            Err(InstallationIssue::Missing)
        );
        std::fs::write(&chrome, "#!/bin/sh\nexec flatpak run com.google.Chrome\n").unwrap();
        assert_eq!(
            discover(Browser::Chrome, &home),
            Err(InstallationIssue::Sandboxed)
        );
        std::fs::remove_file(&chrome).unwrap();
        std::fs::write(&edge, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&edge, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            discover(Browser::Chrome, &home),
            Err(InstallationIssue::DifferentBrowser)
        );
        assert_eq!(discover(Browser::Edge, &home), Ok(edge));
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "browser::tests::isolated_discovery_distinguishes_native_absent_wrapped_and_other_browser", "--nocapture"])
        .env(FIXTURE, root.path()).env("PATH", root.path()).output().unwrap();
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}

#[tokio::test]
async fn explicit_read_waits_for_pending_refresh_instead_of_returning_stale_facts() {
    let detection = Detection::default();
    let slot = detection.0[0].clone();
    slot.state.lock().unwrap().last = Some((
        Facts {
            executable: Err(InstallationIssue::Missing),
            process: ProcessState::Stopped,
            process_ids: Some(Vec::new()),
        },
        Instant::now().checked_sub(Duration::from_secs(3)).unwrap(),
    ));
    // A worker already owns the refresh, but has not published the newly installed browser yet.
    slot.state.lock().unwrap().running = true;
    let read = detection.ready(Browser::Chrome, Path::new("/"));
    tokio::pin!(read);
    let first = std::future::poll_fn(|cx| {
        std::task::Poll::Ready(std::future::Future::poll(read.as_mut(), cx))
    })
    .await;
    assert!(
        first.is_pending(),
        "an explicit read returned the stale missing-browser fact"
    );
    {
        let mut state = slot.state.lock().unwrap();
        state.last = Some((
            Facts {
                executable: Ok(PathBuf::from("/native/google-chrome")),
                process: ProcessState::Running,
                process_ids: Some(vec![123]),
            },
            Instant::now(),
        ));
        state.running = false;
    }
    slot.changed.notify_waiters();
    let facts = read.await.unwrap();
    assert_eq!(facts.executable, Ok(PathBuf::from("/native/google-chrome")));
    assert_eq!(facts.process, ProcessState::Running);
}

#[test]
fn remembered_profile_is_a_single_launch_argument_before_the_address() {
    let command = super::launch::native_command(
        std::path::Path::new("chrome.exe"),
        "https://example.test/",
        Some("Profile 2"),
    );
    assert_eq!(
        command.get_args().collect::<Vec<_>>(),
        ["--profile-directory=Profile 2", "https://example.test/"]
    );
    let unknown = super::launch::native_command(std::path::Path::new("chrome.exe"), "", None);
    assert_eq!(unknown.get_args().count(), 0);
}
