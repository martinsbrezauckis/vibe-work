//! Restart port checks against real sockets after the old listener stops; no Vibe or window needed.
//! Uses the server-free core fixture and keeps the reservation throughout socket teardown.

use super::*;
use std::net::TcpListener;

fn reserved_listener(session: &str) -> (PortReservation, TcpListener) {
    // A free port is found by binding port 0 and letting go of it; another process on a busy machine can take it
    // before the lease binds it again, so a taken port is simply tried again with a fresh one.
    for _ in 0..20 {
        let probe = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);
        let leases = vw_procmon::PortLeases::new(vw_procmon::LeaseConfig {
            first: port,
            last: port,
            per_session: 1,
        });
        if leases.lease(session, &|_| false).is_err() {
            continue;
        }
        let Ok(reservation) = leases.reserve(session, Some(port), &|_| false) else {
            continue;
        };
        if let Ok(listener) = TcpListener::bind(("127.0.0.1", port)) {
            return (reservation, listener);
        }
    }
    panic!("no free loopback port after 20 tries");
}

/// A plain child process that lives about 3 s with no output, started with std's default handle inheritance.
fn outliving_child() -> std::process::Child {
    let mut command = if cfg!(windows) {
        let mut command = std::process::Command::new("ping");
        command.args(["-n", "4", "127.0.0.1"]);
        command
    } else {
        let mut command = std::process::Command::new("sleep");
        command.arg("3");
        command
    };
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap()
}

#[tokio::test]
async fn restart_waits_for_the_stopped_servers_port_to_be_released() {
    let f = crate::feature_test_core::Fixture::new().await;
    let (reservation, old_listener) = reserved_listener("restart");
    let port = reservation.port();
    // The old server stops, but its socket keeps the port busy briefly. The listener itself is that socket:
    // a `try_clone()` copy would be an inheritable handle on Windows (WSADuplicateSocketW), so any child a
    // parallel test spawned meanwhile (git, cmd, rustc) kept the port bound long after this test let go of it.
    // A process the core starts while the old socket is still open (another session's server, a git call) must
    // not keep the port: it outlives the 2 s wait, so an inherited copy would make the restart below fail.
    let mut bystander = outliving_child();
    let release = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(250));
        drop(old_listener);
    });
    let began = Instant::now();
    let result = pick_port(&f.core.shared, "restart", Some(port), Some(reservation));
    let elapsed = began.elapsed();
    release.join().unwrap();
    let _ = bystander.kill();
    let _ = bystander.wait();
    f.finish().await;
    let reservation =
        result.unwrap_or_else(|error| panic!("restart refused after {elapsed:?}: {error}"));
    assert_eq!(reservation.port(), port);
    assert!(elapsed >= Duration::from_millis(200), "{elapsed:?}");
    assert!(elapsed < Duration::from_secs(3), "{elapsed:?}");
}

#[tokio::test]
async fn restart_still_refuses_a_port_that_stays_busy() {
    let f = crate::feature_test_core::Fixture::new().await;
    let (reservation, listener) = reserved_listener("restart-busy");
    let port = reservation.port();
    let began = Instant::now();
    let result = pick_port(
        &f.core.shared,
        "restart-busy",
        Some(port),
        Some(reservation),
    );
    let elapsed = began.elapsed();
    drop(listener);
    f.finish().await;
    let error = result.err().expect("a busy port must still be refused");
    assert_eq!(
        error,
        format!(
            "restart port {port} is not available to bind yet; the old process was stopped; call processes_list and start again on an available leased port"
        )
    );
    assert!(elapsed >= Duration::from_millis(1800), "{elapsed:?}");
    assert!(elapsed < Duration::from_secs(4), "{elapsed:?}");
}
