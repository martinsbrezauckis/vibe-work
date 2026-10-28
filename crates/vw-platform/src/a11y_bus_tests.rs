//! Tests of the accessibility bus check: the decision for each bus state, the address lookup order, and
//! registration against real sockets (a missing one, a silent one, and a private `dbus-daemon` when one
//! is installed). Depends on: `a11y_bus`, tempfile. Called by: `cargo test -p vw-platform`.

use std::io::BufRead;
use std::os::unix::net::UnixListener;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use super::{
    BusState, Registration, advertised_address, bridge_must_be_off, escape_address_value, probe,
    register, session_bus_address,
};

const ADDRESS: &str = "unix:path=/run/user/1/at-spi/bus,guid=0123";

fn probe_advertised(registration: Registration) -> BusState {
    probe(
        false,
        None,
        || Some("unix:path=/session".to_owned()),
        |_| Some(ADDRESS.to_owned()),
        |address| {
            assert_eq!(address, ADDRESS);
            registration
        },
    )
}

#[test]
fn bridge_is_turned_off_only_when_an_advertised_bus_refuses_or_stays_silent() {
    assert!(bridge_must_be_off(&BusState::Refused(
        "Did not receive a reply".to_owned()
    )));
    assert!(bridge_must_be_off(&BusState::NoAnswer));
    assert!(!bridge_must_be_off(&BusState::Registered));
    assert!(!bridge_must_be_off(&BusState::NotAdvertised));
    assert!(!bridge_must_be_off(&BusState::AlreadyOff));
}

#[test]
fn a_working_bus_keeps_the_bridge() {
    assert_eq!(
        probe_advertised(Registration::Accepted),
        BusState::Registered
    );
}

#[test]
fn a_refusing_bus_turns_the_bridge_off() {
    let state = probe_advertised(Registration::Refused("Did not receive a reply".to_owned()));
    assert_eq!(
        state,
        BusState::Refused("Did not receive a reply".to_owned())
    );
    assert!(bridge_must_be_off(&state));
}

#[test]
fn a_registration_timeout_turns_the_bridge_off() {
    let state = probe_advertised(Registration::NoAnswer);
    assert_eq!(state, BusState::NoAnswer);
    assert!(bridge_must_be_off(&state));
}

#[test]
fn no_session_bus_changes_nothing() {
    let state = probe(false, None, || None, |_| unreachable!(), |_| unreachable!());
    assert_eq!(state, BusState::NotAdvertised);
    assert!(!bridge_must_be_off(&state));
}

#[test]
fn no_advertised_bus_or_a_lookup_timeout_changes_nothing() {
    let state = probe(
        false,
        None,
        || Some("unix:path=/session".to_owned()),
        |_| None,
        |_| unreachable!(),
    );
    assert_eq!(state, BusState::NotAdvertised);
    assert!(!bridge_must_be_off(&state));
}

#[test]
fn a_bridge_already_off_is_not_checked() {
    let state = probe(
        true,
        Some(ADDRESS.to_owned()),
        || unreachable!(),
        |_| unreachable!(),
        |_| unreachable!(),
    );
    assert_eq!(state, BusState::AlreadyOff);
    assert!(!bridge_must_be_off(&state));
}

#[test]
fn an_explicit_bus_address_is_checked_without_asking_the_session() {
    let state = probe(
        false,
        Some("unix:path=/explicit".to_owned()),
        || unreachable!(),
        |_| unreachable!(),
        |address| {
            assert_eq!(address, "unix:path=/explicit");
            Registration::Refused("No such file or directory".to_owned())
        },
    );
    assert!(bridge_must_be_off(&state));
}

#[test]
fn the_session_bus_comes_from_the_variable_then_the_runtime_socket() -> std::io::Result<()> {
    let dir = tempfile::tempdir()?;
    let runtime = dir.path().join("run time");
    std::fs::create_dir(&runtime)?;
    let runtime_text = runtime.to_string_lossy().into_owned();
    assert_eq!(
        session_bus_address(Some("unix:path=/x".to_owned()), Some(runtime_text.clone())),
        Some("unix:path=/x".to_owned())
    );
    assert_eq!(session_bus_address(None, Some(runtime_text.clone())), None);
    assert_eq!(session_bus_address(None, None), None);
    let _listener = UnixListener::bind(runtime.join("bus"))?;
    let expected = format!(
        "unix:path={}",
        escape_address_value(&format!("{runtime_text}/bus"))
    );
    assert!(expected.contains("run%20time/bus"));
    assert_eq!(
        session_bus_address(None, Some(runtime_text)),
        Some(expected)
    );
    Ok(())
}

#[test]
fn a_missing_socket_is_refused_at_once() -> std::io::Result<()> {
    let dir = tempfile::tempdir()?;
    let address = format!("unix:path={}/missing", dir.path().display());
    let started = Instant::now();
    assert!(matches!(
        register(&address, Duration::from_secs(5)),
        Registration::Refused(_)
    ));
    assert!(started.elapsed() < Duration::from_secs(2));
    Ok(())
}

#[test]
fn a_silent_socket_times_out() -> std::io::Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("silent");
    // Accepts connections (the kernel's backlog) but never answers.
    let _listener = UnixListener::bind(&path)?;
    let address = format!("unix:path={}", path.display());
    let started = Instant::now();
    assert_eq!(
        register(&address, Duration::from_millis(300)),
        Registration::NoAnswer
    );
    assert!(started.elapsed() < Duration::from_secs(3));
    Ok(())
}

/// A private `dbus-daemon`, stopped on drop.
struct Daemon(Child);

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn start_daemon(dir: &std::path::Path) -> Option<(Daemon, String)> {
    let child = Command::new("dbus-daemon")
        .args(["--session", "--nofork", "--print-address"])
        .arg(format!("--address=unix:path={}", dir.join("bus").display()))
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut daemon = Daemon(child);
    let stdout = daemon.0.stdout.take()?;
    let mut line = String::new();
    std::io::BufReader::new(stdout).read_line(&mut line).ok()?;
    let address = line.trim().to_owned();
    (!address.is_empty()).then_some((daemon, address))
}

#[test]
fn a_real_bus_accepts_its_own_address_and_refuses_another_servers_identity() -> std::io::Result<()>
{
    let dir = tempfile::tempdir()?;
    let Some((_daemon, address)) = start_daemon(dir.path()) else {
        eprintln!("dbus-daemon is not installed: the real-bus registration test did not run");
        return Ok(());
    };
    assert_eq!(
        register(&address, Duration::from_secs(2)),
        Registration::Accepted
    );
    // The same socket under another server's identity: what a replaced bus socket looks like.
    let (path, _) = address.split_once(",guid=").unwrap_or((&address, ""));
    let other = format!("{path},guid=0123456789abcdef0123456789abcdef");
    assert!(matches!(
        register(&other, Duration::from_secs(2)),
        Registration::Refused(_)
    ));
    // This bus runs no accessibility service, and the lookup does not start one.
    assert_eq!(advertised_address(&address, Duration::from_secs(2)), None);
    Ok(())
}
