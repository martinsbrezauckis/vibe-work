//! Exercises the process receipt listener against local clients that never authenticate.
//! Uses real loopback sockets and the native monitor; called by cargo test on every platform.

use super::*;

#[allow(clippy::unwrap_used)]
pub(super) async fn report(address: String, mut receipt: serde_json::Value) -> String {
    // Mirror the launcher's bounded replay of the exact receipt after a dropped
    // connect/write race. A blocking sender also exercises a separate OS thread.
    tokio::task::spawn_blocking(move || {
        use std::io::{Read, Write};
        let token = receipt.as_object_mut().unwrap().remove("token").unwrap();
        let payload = format!("{}\n{receipt}\n", token.as_str().unwrap());
        let deadline = std::time::Instant::now() + Duration::from_millis(900);
        while std::time::Instant::now() < deadline {
            let mut socket = std::net::TcpStream::connect(&address).unwrap();
            socket
                .set_read_timeout(Some(Duration::from_millis(100)))
                .unwrap();
            let mut reply = String::new();
            if socket.write_all(payload.as_bytes()).is_ok()
                && socket.read_to_string(&mut reply).is_ok()
                && !reply.is_empty()
            {
                return reply;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        String::new()
    })
    .await
    .unwrap()
}

#[allow(clippy::unwrap_used)]
async fn fixture() -> (VibeProcesses, vw_procmon::ProcMon, String, String) {
    let monitor = vw_procmon::ProcMon::new(vw_procmon::ProcMonConfig::default()).unwrap();
    let mut config =
        vw_supervisor::SupervisorConfig::new("/unused".into(), "/unused".into(), "/unused".into());
    let tracker = VibeProcesses::start(&mut config, &monitor).await.unwrap();
    let endpoint = config.extra_env.last().unwrap().1.to_str().unwrap();
    let (port, token) = endpoint.split_once(':').unwrap();
    (tracker, monitor, format!("127.0.0.1:{port}"), token.into())
}

#[tokio::test]
async fn idle_and_slow_clients_cannot_delay_a_real_receipt() {
    let (tracker, _monitor, address, token) = fixture().await;
    let mut attackers = Vec::new();
    for index in 0..64 {
        let mut socket = TcpStream::connect(&address).await.unwrap();
        if index % 2 == 0 {
            let _ = socket.write_all(b"{").await;
        }
        attackers.push(socket);
    }
    let target = address.clone();
    let churn = tokio::spawn(async move {
        loop {
            let mut socket = TcpStream::connect(&target).await.unwrap();
            let _ = socket.write_all(b"partial").await;
            tokio::task::yield_now().await;
        }
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    let receipt = serde_json::json!({"token":token, "session":"flood", "id":"real",
        "command":"native identity", "pid":std::process::id(),
        "startToken":vw_procmon::identity(std::process::id()).unwrap().start.0});
    let started = std::time::Instant::now();
    let result = tokio::time::timeout(Duration::from_secs(1), report(address, receipt)).await;
    churn.abort();
    assert_eq!(
        result.expect("idle/slow clients delayed a real receipt beyond one second"),
        "ok\n"
    );
    assert_eq!(tracker.live("flood").len(), 1);
    eprintln!(
        "real receipt under 64 idle/slow clients: {:?}",
        started.elapsed()
    );
    drop(attackers);
}

#[tokio::test]
async fn oversized_first_line_is_dropped_without_waiting_for_newline() {
    use std::io::Write;
    let (_tracker, monitor, address, token) = fixture().await;
    // Queue bytes before yielding to the listener: this must reject the oversized
    // header itself, rather than accidentally exercise an empty arrival race.
    let mut socket = std::net::TcpStream::connect(&address).unwrap();
    socket
        .write_all(format!("{token}{}", "x".repeat(4097)).as_bytes())
        .unwrap();
    socket.set_nonblocking(true).unwrap();
    let mut socket = TcpStream::from_std(socket).unwrap();
    let result = tokio::time::timeout(Duration::from_millis(500), socket.read_u8()).await;
    assert!(
        matches!(result, Ok(Err(_))),
        "oversized first line held a client open: {result:?}"
    );
    assert_eq!(monitor.registry().list(), []);
}

#[tokio::test]
async fn idle_and_fragmented_headers_are_closed_before_the_auth_deadline() {
    use std::io::Write;
    let (_tracker, monitor, address, token) = fixture().await;
    for prefix in ["", &token[..32], &token] {
        let mut socket = std::net::TcpStream::connect(&address).unwrap();
        socket.write_all(prefix.as_bytes()).unwrap();
        socket.set_nonblocking(true).unwrap();
        let mut socket = TcpStream::from_std(socket).unwrap();
        let result = tokio::time::timeout(Duration::from_millis(250), socket.read_u8()).await;
        assert!(
            matches!(result, Ok(Err(_))),
            "partial header stayed open: {result:?}"
        );
    }
    assert_eq!(monitor.registry().list(), []);
}

#[tokio::test]
async fn wrong_tokens_cannot_register_even_with_a_complete_receipt() {
    use std::io::Write;
    let (_tracker, monitor, address, token) = fixture().await;
    for index in [0, 31, 63] {
        let mut wrong = token.as_bytes().to_vec();
        wrong[index] = if wrong[index] == b'a' { b'b' } else { b'a' };
        let receipt = serde_json::json!({"session":"wrong", "id":"bad",
            "command":"not owned", "pid":std::process::id()});
        wrong.extend_from_slice(format!("\n{receipt}\n").as_bytes());
        let mut socket = std::net::TcpStream::connect(&address).unwrap();
        socket.write_all(&wrong).unwrap();
        socket.set_nonblocking(true).unwrap();
        let mut socket = TcpStream::from_std(socket).unwrap();
        let result = tokio::time::timeout(Duration::from_millis(250), socket.read_u8()).await;
        assert!(
            matches!(result, Ok(Err(_))),
            "wrong token was admitted: {result:?}"
        );
    }
    assert_eq!(monitor.registry().list(), []);
}

#[tokio::test]
async fn receipts_without_or_with_another_start_token_cannot_register() {
    let (tracker, monitor, address, token) = fixture().await;
    let process = vw_procmon::identity(std::process::id()).unwrap();
    let mut receipt = serde_json::json!({"token":token, "session":"identity", "id":"real",
        "command":"native identity", "pid":process.pid});
    assert_eq!(report(address.clone(), receipt.clone()).await, "");
    receipt["startToken"] = (process.start.0 + 1).into();
    assert_eq!(report(address.clone(), receipt.clone()).await, "gone\n");
    assert_eq!(monitor.registry().list(), []);
    receipt["startToken"] = process.start.0.into();
    assert_eq!(report(address.clone(), receipt.clone()).await, "ok\n");
    receipt["startToken"] = (process.start.0 + 1).into();
    assert_eq!(report(address, receipt).await, "gone\n");
    assert_eq!(
        monitor.registry().list().len(),
        1,
        "stale replay must not erase valid ownership"
    );
    assert_eq!(tracker.live("identity")[0].process, process);
}
