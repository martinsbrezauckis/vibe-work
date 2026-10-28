//! Request deadlines against a child that holds its pipes open without reading stdin.
//! Uses the Rust test executable as a disposable helper; called by the pipe unit tests.
use super::*;

#[test]
#[ignore = "child fixture: run only by the request deadline test"]
fn holds_stdin_open_without_reading() {
    std::thread::sleep(Duration::from_secs(120));
}

// Unix: a paused clock, so the 35 s deadline passes at once. Windows: tokio writes a child's stdin on a blocking
// thread, and a paused clock never moves while that write blocks, so there the test runs the 35 s in real time.
#[cfg_attr(not(windows), tokio::test(start_paused = true))]
#[cfg_attr(windows, tokio::test)]
async fn request_deadline_includes_a_blocked_stdin_write() {
    // The helper holds stdin open without reading it. On Windows it must also print nothing: tokio hands the write
    // to a blocking thread and the request goes on to read, where the libtest child's "running 1 test" header would
    // be taken for a reply. There a silent PowerShell `Start-Sleep` is the helper.
    let mut command = if cfg!(windows) {
        let mut command = tokio::process::Command::new("powershell");
        command.args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Start-Sleep -Seconds 120",
        ]);
        command
    } else {
        let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
        command.args([
            "--exact",
            "chrome_control::pipe::tests::holds_stdin_open_without_reading",
            "--ignored",
        ]);
        command
    };
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let input = child.stdin.take().unwrap();
    let output = BufReader::new(child.stdout.take().unwrap().take(1024 * 1024 + 2));
    let mut pipe = Pipe {
        child,
        _executable: None,
        input: Some(input),
        output,
        id: 0,
        shared: Weak::new(),
    };
    let started = tokio::time::Instant::now();
    // Below the request cap, above an OS pipe's capacity: the helper never drains it.
    let result = tokio::time::timeout(
        Duration::from_secs(36),
        pipe.rpc("tools/call", json!({"payload": "x".repeat(512 * 1024)})),
    )
    .await;
    pipe.child.kill().await.unwrap();
    assert_eq!(
        result,
        Ok(Err(())),
        "the write must share the reply deadline"
    );
    if cfg!(windows) {
        // Real time: the deadline, plus the moment the kill and the timer take.
        let elapsed = started.elapsed();
        assert!(
            (Duration::from_secs(35)..Duration::from_secs(36)).contains(&elapsed),
            "{elapsed:?}"
        );
    } else {
        assert_eq!(started.elapsed(), Duration::from_secs(35));
    }
}
