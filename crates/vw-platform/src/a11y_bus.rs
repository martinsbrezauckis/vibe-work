//! Startup check of the desktop accessibility bus (Linux only): can this process register on the
//! accessibility (AT-SPI) bus that the session advertises? The check works the way GTK's accessibility
//! bridge registers: libdbus, which also checks that the server is the one the address names (its
//! `guid=`). If the session advertises a bus but registration there is refused or gets no answer,
//! the bridge is turned off for this run (`NO_AT_BRIDGE=1` in this process's environment, before GTK
//! starts). In that state the bridge's own start fails anyway, and the web view's later accessibility
//! hand-over can then end the process. In every other case nothing changes, so screen readers keep
//! working whenever the bus works.
//!
//! - The address comes from where the bridge looks: `AT_SPI_BUS_ADDRESS`, else the session bus's
//!   `org.a11y.Bus.GetAddress` (asked without starting the service: only a running one can be in this
//!   state). An address published only as an X11 root-window property is not read; the session bus
//!   gives the same address when one launcher serves both.
//! - Each of the two steps (the lookup, the registration) has a deadline ([`STEP_TIMEOUT`]), also against a
//!   socket that accepts but never answers. A lookup that fails or runs out of time counts as "no bus
//!   advertised" (nothing changes); only a known address that refuses us or stays silent turns the bridge off.
//! - The environment is changed only while the process has a single thread (read from
//!   `/proc/self/task`), which is what makes the change sound.
//! - Next to it the check sets [`crate::child_env::BRIDGE_SET_BY_APP_VAR`], so programs this run starts (a
//!   terminal's shell, the file manager, the browser) get neither: [`crate::child_env`]. Nothing is written
//!   outside the process.
//!
//! Depends on: dbus (libdbus). Called by: the app as the first step of its start, before GTK starts.

use std::time::{Duration, Instant};

use dbus::Message;
use dbus::channel::Channel;

/// The deadline for each step of the check (the address lookup, the registration).
pub const STEP_TIMEOUT: Duration = Duration::from_secs(1);

/// What the check found on the accessibility bus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BusState {
    /// `NO_AT_BRIDGE` was already set: the bridge is off and no check runs.
    AlreadyOff,
    /// No session bus, or it advertises no accessibility bus (or did not say in time).
    NotAdvertised,
    /// This process registered on the advertised bus: the bridge will too.
    Registered,
    /// The advertised bus refused the registration (the reason libdbus gave).
    Refused(String),
    /// The advertised bus did not answer the registration within [`STEP_TIMEOUT`].
    NoAnswer,
}

/// The check's result, for the caller to log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct A11yBusCheck {
    /// What the check found.
    pub state: BusState,
    /// `NO_AT_BRIDGE=1` was set for this run.
    pub bridge_turned_off: bool,
    /// The bridge should be off but the environment was left alone (the process already had threads).
    pub left_on_threads: bool,
    /// How long the check took.
    pub elapsed: Duration,
}

/// The outcome of one registration attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Registration {
    /// The bus answered `Hello`.
    Accepted,
    /// The connection or `Hello` failed before the timeout.
    Refused(String),
    /// No answer within the timeout.
    NoAnswer,
}

/// Whether a bus state means the bridge must be off for this run: only a known address that refuses us
/// or does not answer.
#[must_use]
pub fn bridge_must_be_off(state: &BusState) -> bool {
    matches!(state, BusState::Refused(_) | BusState::NoAnswer)
}

/// Runs the check and, when [`bridge_must_be_off`], sets `NO_AT_BRIDGE=1`. Never fails. Call it before
/// GTK starts and before any other thread exists.
#[must_use]
pub fn check_accessibility_bus() -> A11yBusCheck {
    let started = Instant::now();
    let env = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
    let state = probe(
        std::env::var_os("NO_AT_BRIDGE").is_some(),
        env("AT_SPI_BUS_ADDRESS"),
        || session_bus_address(env("DBUS_SESSION_BUS_ADDRESS"), env("XDG_RUNTIME_DIR")),
        |session| advertised_address(session, STEP_TIMEOUT),
        |address| register(address, STEP_TIMEOUT),
    );
    let must_be_off = bridge_must_be_off(&state);
    let bridge_turned_off = must_be_off && turn_bridge_off();
    A11yBusCheck {
        state,
        bridge_turned_off,
        left_on_threads: must_be_off && !bridge_turned_off,
        elapsed: started.elapsed(),
    }
}

/// The check's decision steps, with the bus calls passed in (the tests give their own).
pub fn probe(
    already_off: bool,
    explicit_address: Option<String>,
    session: impl FnOnce() -> Option<String>,
    lookup: impl FnOnce(&str) -> Option<String>,
    register: impl FnOnce(&str) -> Registration,
) -> BusState {
    if already_off {
        return BusState::AlreadyOff;
    }
    let address = match explicit_address {
        Some(address) => address,
        None => match session().as_deref().and_then(lookup) {
            Some(address) => address,
            None => return BusState::NotAdvertised,
        },
    };
    match register(&address) {
        Registration::Accepted => BusState::Registered,
        Registration::Refused(reason) => BusState::Refused(reason),
        Registration::NoAnswer => BusState::NoAnswer,
    }
}

/// The session bus address the way libdbus finds it, without its X11 autolaunch (which would start a
/// new bus): the variable, else the per-user socket in the runtime folder.
#[must_use]
pub fn session_bus_address(
    variable: Option<String>,
    runtime_dir: Option<String>,
) -> Option<String> {
    use std::os::unix::fs::FileTypeExt;
    if variable.is_some() {
        return variable;
    }
    let socket = std::path::Path::new(&runtime_dir?).join("bus");
    let is_socket = std::fs::metadata(&socket).is_ok_and(|meta| meta.file_type().is_socket());
    is_socket.then(|| {
        format!(
            "unix:path={}",
            escape_address_value(&socket.to_string_lossy())
        )
    })
}

/// D-Bus address value escaping: bytes outside `[-0-9A-Za-z_/.\*]` become `%xx`.
fn escape_address_value(value: &str) -> String {
    use std::fmt::Write;
    let mut escaped = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_/.\\*".contains(&byte) {
            escaped.push(char::from(byte));
        } else {
            let _ = write!(escaped, "%{byte:02x}");
        }
    }
    escaped
}

/// Asks the session bus for the accessibility bus address, without starting the service.
fn advertised_address(session: &str, timeout: Duration) -> Option<String> {
    let deadline = Instant::now() + timeout;
    let channel = connect(session, deadline).ok()?;
    let mut get = Message::new_method_call(
        "org.a11y.Bus",
        "/org/a11y/bus",
        "org.a11y.Bus",
        "GetAddress",
    )
    .ok()?;
    get.set_auto_start(false);
    let reply = call(&channel, get, deadline).ok()?;
    reply
        .read1::<&str>()
        .ok()
        .filter(|address| !address.is_empty())
        .map(str::to_owned)
}

/// Registers on `address` within `timeout`, as the bridge does. libdbus closes the connection to a server
/// whose identity does not match the address's `guid=`, which shows here as a refusal.
#[must_use]
pub fn register(address: &str, timeout: Duration) -> Registration {
    match connect(address, Instant::now() + timeout) {
        Ok(_channel) => Registration::Accepted,
        Err(Failure::Refused(reason)) => Registration::Refused(reason),
        Err(Failure::NoAnswer) => Registration::NoAnswer,
    }
}

/// Why a call got no reply.
enum Failure {
    Refused(String),
    NoAnswer,
}

/// Opens a private connection and registers on the bus (`Hello`) before `deadline`.
fn connect(address: &str, deadline: Instant) -> Result<Channel, Failure> {
    let channel =
        Channel::open_private(address).map_err(|error| Failure::Refused(reason(&error)))?;
    let hello = Message::new_method_call(
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "Hello",
    )
    .map_err(Failure::Refused)?;
    call(&channel, hello, deadline)?;
    Ok(channel)
}

/// Sends `message` and waits for its reply until `deadline`. The connection is driven here, one bounded
/// read/write at a time: libdbus's own blocking call does not apply its timeout while the connection is
/// still authenticating, so a socket that accepts but never answers would hold it indefinitely.
fn call(channel: &Channel, message: Message, deadline: Instant) -> Result<Message, Failure> {
    let serial = channel
        .send(message)
        .map_err(|()| Failure::Refused("the message could not be queued".to_owned()))?;
    loop {
        while let Some(mut incoming) = channel.pop_message() {
            if incoming.get_reply_serial() == Some(serial) {
                return match incoming.as_result() {
                    Ok(reply) => Ok(reply.duplicate().map_err(Failure::Refused)?),
                    Err(error) => Err(Failure::Refused(reason(&error))),
                };
            }
        }
        if !channel.is_connected() {
            return Err(Failure::Refused("the bus closed the connection".to_owned()));
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(Failure::NoAnswer);
        }
        if channel
            .read_write(Some(left.min(Duration::from_millis(100))))
            .is_err()
        {
            return Err(Failure::Refused("the bus closed the connection".to_owned()));
        }
    }
}

fn reason(error: &dbus::Error) -> String {
    error
        .message()
        .or(error.name())
        .unwrap_or("no reason given")
        .to_owned()
}

/// Sets `NO_AT_BRIDGE=1` and the marker that this run set it if the process has a single thread; returns
/// whether it did.
#[allow(unsafe_code)]
fn turn_bridge_off() -> bool {
    let threads = std::fs::read_dir("/proc/self/task").map(Iterator::count);
    if threads.ok() != Some(1) {
        return false;
    }
    // SAFETY: the process has one thread (checked just above), so no other thread can read or write the
    // environment at the same time.
    unsafe {
        std::env::set_var(crate::child_env::BRIDGE_VAR, "1");
        std::env::set_var(crate::child_env::BRIDGE_SET_BY_APP_VAR, "1");
    };
    true
}

#[cfg(test)]
#[path = "a11y_bus_tests.rs"]
mod tests;
