//! Runs the production Chrome launcher with a disposable executable, in an isolated subprocess PATH.
//! The launcher exits immediately; /proc must show no zombie while the app/test process is still alive.
use super::*;
use std::os::unix::fs::PermissionsExt;
#[test]
fn chrome_launch_reaps_its_child() {
    const CHILD: &str = "VW_CHROME_REAP_FIXTURE";
    if let Some(pid_file) = std::env::var_os(CHILD) {
        open(&ChromeControlConfig::default(), "https://example.test").unwrap();
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
