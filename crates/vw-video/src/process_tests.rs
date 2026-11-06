//! Cross-platform supervision regressions use a child Rust test process, independent of shell/runtime codecs.
use super::*;

fn child(mode: &str, pid: Option<&std::path::Path>) -> CommandWrap {
    CommandWrap::with_new(std::env::current_exe().unwrap(), |c| {
        c.args([
            "--exact",
            "process::tests::helper_child",
            "--ignored",
            "--nocapture",
        ])
        .env("VW_VIDEO_CHILD_MODE", mode);
        if let Some(path) = pid {
            c.env("VW_VIDEO_CHILD_PID", path);
        }
    })
}

#[test]
#[ignore = "entry point invoked only by the supervision parents"]
fn helper_child() {
    let Ok(mode) = std::env::var("VW_VIDEO_CHILD_MODE") else {
        return;
    };
    if let Some(path) = std::env::var_os("VW_VIDEO_CHILD_PID") {
        std::fs::write(path, std::process::id().to_string()).unwrap();
    }
    #[cfg(windows)]
    if mode == "quota" {
        // The exit code Windows gives a process ended at its Job's CPU-time limit.
        std::process::exit(0xC000_0044_u32.cast_signed());
    }
    if mode == "flood" {
        use std::io::Write;
        let buffer = vec![b'!'; 64 * 1024];
        let mut out = std::io::stdout().lock();
        loop {
            if out.write_all(&buffer).is_err() {
                break;
            }
        }
    } else {
        std::thread::sleep(Duration::from_secs(60));
    }
}

#[tokio::test]
async fn helper_deadline_is_external() {
    assert_eq!(
        capture(child("hang", None), Duration::from_millis(50), Vec::new())
            .await
            .unwrap_err(),
        "timeout: video helper deadline"
    );
}

#[tokio::test]
async fn helper_output_flood_is_bounded() {
    assert_eq!(
        capture(child("flood", None), Duration::from_secs(5), Vec::new())
            .await
            .unwrap_err(),
        "limit_exceeded: helper output"
    );
}

fn exists(pid: u32) -> bool {
    #[cfg(target_os = "linux")]
    {
        std::path::Path::new("/proc").join(pid.to_string()).exists()
    }
    #[cfg(not(target_os = "linux"))]
    {
        vw_platform::process_facts(pid).is_some()
    }
}

#[tokio::test]
async fn cancelled_helper_is_killed_and_reaped() {
    let dir = tempfile::tempdir().unwrap();
    let pidfile = dir.path().join("pid");
    let task = tokio::spawn(capture(
        child("hang", Some(&pidfile)),
        Duration::from_secs(35),
        Vec::new(),
    ));
    let started = std::time::Instant::now();
    while !pidfile.exists() && started.elapsed() < Duration::from_secs(5) {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let pid = std::fs::read_to_string(pidfile).unwrap().parse().unwrap();
    assert!(exists(pid), "worker must start before cancellation");
    task.abort();
    let _ = task.await;
    let started = std::time::Instant::now();
    while exists(pid) && started.elapsed() < Duration::from_secs(5) {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(!exists(pid), "cancelled helper remains alive or unreaped");
}

#[cfg(windows)]
#[tokio::test]
async fn cpu_time_limit_is_a_named_timeout() {
    assert_eq!(
        capture(child("quota", None), Duration::from_secs(5), Vec::new())
            .await
            .unwrap_err(),
        "timeout: video worker CPU time limit"
    );
}
