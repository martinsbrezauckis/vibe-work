//! Runs the production Chrome launcher with a disposable executable, in an isolated subprocess PATH.
//! The launcher exits immediately; /proc must show no zombie while the app/test process is still alive.
use super::*;
use std::os::unix::fs::PermissionsExt;
#[test]
fn chrome_launch_survives_its_parent_in_a_detached_session() {
    const CHILD: &str = "VW_CHROME_DETACHED_FIXTURE";
    if let Some(marker) = std::env::var_os(CHILD) {
        open(&ChromeControlConfig::default(), "").unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while !std::path::Path::new(&marker).exists() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let launcher = dir.path().join("google-chrome");
    std::fs::write(
        &launcher,
        "#!/bin/sh\nprintf '%s' \"$$\" > \"$VW_CHROME_DETACHED_FIXTURE\"\nexec /bin/sleep 30\n",
    )
    .unwrap();
    std::fs::set_permissions(&launcher, std::fs::Permissions::from_mode(0o700)).unwrap();
    let marker = dir.path().join("pid");
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "chrome_control::browser::reap_tests::chrome_launch_survives_its_parent_in_a_detached_session"])
        .env(CHILD, &marker).env("PATH", dir.path()).output().unwrap();
    assert!(result.status.success(), "{result:?}");
    let pid = std::fs::read_to_string(marker).unwrap();
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
    // Stop only the disposable child after recording its survival and session identity.
    let _ = std::process::Command::new("/bin/kill")
        .arg(&pid)
        .status()
        .unwrap();
    let fields: Vec<_> = stat.split_whitespace().collect();
    assert_ne!(fields[2], "Z", "the browser outlives its launching app");
    assert_eq!(
        fields[5], pid,
        "browser must be in a detached OS session: {stat}"
    );
}

#[test]
fn chrome_launch_reaps_its_child() {
    const CHILD: &str = "VW_CHROME_REAP_FIXTURE";
    if let Some(pid_file) = std::env::var_os(CHILD) {
        open(&ChromeControlConfig::default(), "https://example.test").unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        let pid = loop {
            // The shell creates the file before it writes the PID: an empty read is not a PID yet.
            if let Ok(pid) = std::fs::read_to_string(&pid_file)
                && !pid.trim().is_empty()
            {
                break pid.trim().to_owned();
            }
            assert!(std::time::Instant::now() < deadline, "launcher never ran");
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        let proc = PathBuf::from(format!("/proc/{pid}/stat"));
        while proc.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(
            !proc.exists(),
            "Chrome child remains (Z is <defunct>): {:?}",
            std::fs::read_to_string(proc)
        );
        return;
    }
    let dir = std::env::var_os("VW_TEST_ROOT")
        .map_or_else(std::env::temp_dir, PathBuf::from)
        .join(format!("chrome-reap-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let launcher = dir.join("google-chrome");
    std::fs::write(
        &launcher,
        "#!/bin/sh\nprintf '%s' \"$$\" > \"$VW_CHROME_REAP_FIXTURE\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&launcher, std::fs::Permissions::from_mode(0o700)).unwrap();
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "chrome_control::browser::reap_tests::chrome_launch_reaps_its_child",
            "--nocapture",
        ])
        .env(CHILD, dir.join("pid"))
        .env("PATH", &dir)
        .output()
        .unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}

#[cfg(feature = "your-edge")]
#[test]
fn edge_store_launch_reaps_its_child() {
    const CHILD: &str = "VW_EDGE_REAP_FIXTURE";
    if let Some(pid_file) = std::env::var_os(CHILD) {
        super::super::setup::open_store_for(&ChromeControlConfig::default(), true).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        let pid = loop {
            if let Ok(pid) = std::fs::read_to_string(&pid_file) {
                break pid.trim().to_owned();
            }
            assert!(std::time::Instant::now() < deadline, "launcher never ran");
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        let proc = PathBuf::from(format!("/proc/{pid}/stat"));
        while proc.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(
            !proc.exists(),
            "Edge store child remains: {:?}",
            std::fs::read_to_string(proc)
        );
        return;
    }
    let dir = std::env::var_os("VW_TEST_ROOT")
        .map_or_else(std::env::temp_dir, PathBuf::from)
        .join(format!("edge-reap-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let launcher = dir.join("microsoft-edge");
    std::fs::write(
        &launcher,
        "#!/bin/sh\nprintf '%s' \"$$\" > \"$VW_EDGE_REAP_FIXTURE\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&launcher, std::fs::Permissions::from_mode(0o700)).unwrap();
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "chrome_control::browser::reap_tests::edge_store_launch_reaps_its_child",
            "--nocapture",
        ])
        .env(CHILD, dir.join("pid"))
        .env("PATH", &dir)
        .output()
        .unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}

#[tokio::test(flavor = "current_thread")]
async fn process_detection_never_blocks_the_async_worker() {
    const CHILD: &str = "VW_BROWSER_SLOW_PROBE";
    if let Some(pid_file) = std::env::var_os(CHILD) {
        let _ = running(&ChromeControlConfig::default());
        let pid_file = PathBuf::from(pid_file);
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        let pid = loop {
            if let Ok(pid) = std::fs::read_to_string(&pid_file)
                && !pid.is_empty()
            {
                break pid;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "background probe never started"
            );
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        };
        let process = PathBuf::from(format!("/proc/{}/stat", pid.trim()));
        let held = process.exists();
        // Only this async worker can release the helper; status must return while it is held.
        std::fs::write(pid_file.with_extension("release"), b"release").unwrap();
        while process.exists() {
            assert!(
                tokio::time::Instant::now() < deadline,
                "released probe {pid} was not reaped"
            );
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(
            held,
            "status waited for the held helper instead of returning cached facts"
        );
        return;
    }
    let dir = std::env::var_os("VW_TEST_ROOT")
        .map_or_else(std::env::temp_dir, PathBuf::from)
        .join(format!("browser-slow-probe-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let probe = dir.join("pgrep");
    std::fs::write(
        &probe,
        "#!/bin/sh\nprintf '%s' \"$$\" > \"$VW_BROWSER_SLOW_PROBE\"\nwhile [ ! -f \"$VW_BROWSER_SLOW_PROBE.release\" ]; do /bin/sleep 0.02; done\n",
    )
    .unwrap();
    std::fs::set_permissions(&probe, std::fs::Permissions::from_mode(0o700)).unwrap();
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "chrome_control::browser::reap_tests::process_detection_never_blocks_the_async_worker",
            "--nocapture",
        ])
        .env(CHILD, dir.join("pid"))
        .env("PATH", &dir)
        .output()
        .unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}
