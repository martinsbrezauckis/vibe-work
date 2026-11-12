//! Request deadlines against a child that holds its pipes open without reading stdin.
//! Uses the Rust test executable as a disposable helper; called by the pipe unit tests.
use super::*;

#[test]
#[ignore = "child fixture: run only by the request deadline test"]
fn holds_stdin_open_without_reading() {
    std::thread::sleep(Duration::from_secs(120));
}

// A helper that never reads its stdin cannot hold the dispatcher: each request write is bounded to 500 ms.
// - Unix (paused clock): the first request, larger than the pipe, fails at exactly 500 ms.
// - Windows (real time): the child's stdin takes a write into its own buffer at once and writes it to the pipe in
//   the background, so the first request is accepted without waiting; the next one waits on that unfinished write
//   and fails at the 500 ms bound. A helper that stays stuck after an accepted request is ended by the 35 s
//   request deadline (`Pipe::rpc`).
#[cfg_attr(not(windows), tokio::test(start_paused = true))]
#[cfg_attr(windows, tokio::test)]
async fn dispatch_write_cannot_block_the_supervisor() {
    // The helper holds stdin open without reading it. On Windows it must also print nothing: tokio hands the write
    // to a blocking thread and the request goes on to read, where the libtest child's "running 1 test" header would
    // be taken for a reply. There the helper is `cmd /c ping` with its output discarded: neither reads stdin
    // (PowerShell drains a redirected stdin, so the pipe never filled and the write was never bounded).
    let mut command = if cfg!(windows) {
        let mut command = tokio::process::Command::new("cmd");
        command.args(["/d", "/c", "ping -n 121 127.0.0.1 >nul"]);
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
        read_buffer: Vec::new(),
        shared: Weak::new(),
        tolerant: std::collections::HashSet::new(),
    };
    // Below the request cap, above an OS pipe's capacity: the helper never drains it.
    let payload = || json!({"payload": "x".repeat(512 * 1024)});
    let started = tokio::time::Instant::now();
    let first =
        tokio::time::timeout(Duration::from_secs(1), pipe.begin("tools/call", payload())).await;
    let first_took = started.elapsed();
    let second = if cfg!(windows) {
        let started = tokio::time::Instant::now();
        let second =
            tokio::time::timeout(Duration::from_secs(1), pipe.begin("tools/call", payload())).await;
        Some((second, started.elapsed()))
    } else {
        None
    };
    assert!(
        pipe.child.try_wait().unwrap().is_none(),
        "requests finished while the helper still held stdin without reading it"
    );
    // `cmd` runs ping as its child, which holds the same stdin: end the whole tree so the background write ends.
    if cfg!(windows)
        && let Some(pid) = pipe.child.id()
    {
        let _ = tokio::process::Command::new("taskkill")
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;
    }
    let _ = pipe.child.kill().await;
    if let Some((second, second_took)) = second {
        assert_eq!(first, Ok(Ok(1)), "the first request is accepted");
        assert_eq!(
            second,
            Ok(Err(())),
            "the dispatcher must bound the write independently from a reply"
        );
        // Real time: the bound, plus the moment the timer takes.
        assert!(
            (Duration::from_millis(500)..Duration::from_secs(1)).contains(&second_took),
            "{second_took:?}"
        );
    } else {
        assert_eq!(
            first,
            Ok(Err(())),
            "the dispatcher must bound the write independently from a reply"
        );
        assert_eq!(first_took, Duration::from_millis(500));
    }
}

#[cfg(unix)]
#[tokio::test]
async fn concurrent_replies_keep_identity_and_partial_reads_survive_cancellation() {
    let script = r#"IFS= read -r first
IFS= read -r second
printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"chat":"second"}}'
printf '%s' '{"jsonrpc":"2.0","id":1,'
sleep 0.2
printf '%s\n' '"result":{"chat":"first"}}'
"#;
    let mut child = tokio::process::Command::new("/bin/sh")
        .args(["-c", script])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let input = child.stdin.take();
    let output = BufReader::new(child.stdout.take().unwrap().take(1024 * 1024 + 2));
    let mut pipe = Pipe {
        child,
        _executable: None,
        input,
        output,
        id: 0,
        read_buffer: Vec::new(),
        shared: Weak::new(),
        tolerant: std::collections::HashSet::new(),
    };
    assert_eq!(
        pipe.begin("tools/call", json!({"chat":"first"})).await,
        Ok(1)
    );
    assert_eq!(
        pipe.begin("tools/call", json!({"chat":"second"})).await,
        Ok(2)
    );
    assert_eq!(pipe.next().await, Ok((2, json!({"chat":"second"}))));
    assert!(
        tokio::time::timeout(Duration::from_millis(20), pipe.next())
            .await
            .is_err()
    );
    assert_ne!(pipe.read_buffer, [] as [u8; 0]);
    assert_eq!(pipe.next().await, Ok((1, json!({"chat":"first"}))));
    pipe.close().await;
}

/// An older helper answers an enrollment request with "method not found": that answer is an outcome for that
/// request only. The same error answer to a tool call still ends the connection.
#[cfg(unix)]
#[tokio::test]
async fn only_enrollment_requests_take_an_error_answer() {
    let script = r#"IFS= read -r first
IFS= read -r second
printf '%s\n' '{"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"Method not found"}}'
printf '%s\n' '{"jsonrpc":"2.0","id":2,"error":{"code":-32601,"message":"Method not found"}}'
"#;
    let mut child = tokio::process::Command::new("/bin/sh")
        .args(["-c", script])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let input = child.stdin.take();
    let output = BufReader::new(child.stdout.take().unwrap().take(1024 * 1024 + 2));
    let mut pipe = Pipe {
        child,
        _executable: None,
        input,
        output,
        id: 0,
        read_buffer: Vec::new(),
        shared: Weak::new(),
        tolerant: std::collections::HashSet::new(),
    };
    assert_eq!(
        pipe.begin_tolerant("owner.enroll.mint", json!({})).await,
        Ok(1)
    );
    assert_eq!(pipe.begin("tools/call", json!({})).await, Ok(2));
    assert_eq!(
        pipe.next().await,
        Ok((
            1,
            json!({"rpcError":{"code":-32601,"message":"Method not found"}})
        ))
    );
    assert_eq!(pipe.next().await, Err(()));
    pipe.close().await;
}
