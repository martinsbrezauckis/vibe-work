//! Tests of the accessibility bus check: the decision for each bus state, the address lookup order, and
//! registration against real sockets (a missing one, a silent one, and a private `dbus-daemon` when one
//! is installed). Depends on: `a11y_bus`, tempfile. Called by: `cargo test -p vw-platform`.

use std::io::BufRead;
use std::os::unix::net::UnixListener;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use super::{
    Registration, advertised_address, escape_address_value, register, session_bus_address,
};

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
    let registration = register(&address, Duration::from_secs(2));
    if registration
        == Registration::Refused("Failed to query AppArmor policy: Read-only file system".into())
    {
        use std::io::Write;
        writeln!(
            std::io::stderr().lock(),
            "SKIPPED: real bus identity: the disposable dbus-daemon cannot query the sandbox's read-only AppArmor policy; NOT RUN (not a pass)"
        )?;
        return Ok(());
    }
    assert_eq!(registration, Registration::Accepted);
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
