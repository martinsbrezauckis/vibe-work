//! One app per data folder, acquired by Tauri before starting the host.
//! Uses a persistent OS-locked file and a loopback focus endpoint; different data folders remain independent.
//! The lock is never unlinked: a crash releases it without letting contenders lock different inodes.
//! A second launch may carry a hand-over ("Send to Vibe Work", [`crate::hand_over`]): it is written after the
//! endpoint's token, the running app admits it again and gives it to its `deliver` seam, then focuses.
//! Request: the 64-character token, then `F` (focus) or `H` + a 4-byte big-endian length + the hand-over's JSON.
//! Reply: one byte, `1` when the hand-over was delivered or the focus-only request focused the window.
//! Restart: the app writes [`RESTART_MARK`] before it relaunches itself; the new process then waits for the old
//! owner's lock instead of focusing the closing window or refusing the folder, and removes the mark once it owns it.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::hand_over::{HandOver, MAX_WIRE_BYTES};

const WAIT: Duration = Duration::from_secs(2);
/// How long a second launch waits for a starting owner to publish its endpoint (several file manager entries
/// may start at once, one process per file).
const STARTING: Duration = Duration::from_secs(10);
/// The data-folder file that marks the next launch as this app's own restart (epoch milliseconds of the request).
pub const RESTART_MARK: &str = "instance.restart";
/// A mark older than this is a relaunch that never came; it is ignored.
const RESTART_FRESH: Duration = Duration::from_secs(30);
/// How long the relaunched process waits for the closing owner to release the folder.
const RESTART_WAIT: Duration = Duration::from_secs(15);

#[derive(Serialize, Deserialize)]
struct Endpoint {
    port: u16,
    instance: String,
}

/// The outcome of launching into a data folder.
pub enum InstanceStart {
    /// This process owns the folder until the guard is dropped.
    First(InstanceGuard),
    /// Another process owns it; whether it accepted this launch's request.
    Existing {
        /// Whether the hand-over was delivered, or the focus-only request focused the window.
        focused: bool,
    },
}

/// Keeps the folder lock and focus listener alive for the entire app lifetime.
pub struct InstanceGuard {
    _lock: File,
    stop: Arc<AtomicBool>,
    wake: SocketAddr,
    listener: Option<std::thread::JoinHandle<()>>,
}

impl InstanceGuard {
    /// The port of this instance's local listener (a port of Vibe Work itself: never reachable from a browser page).
    #[must_use]
    pub fn port(&self) -> u16 {
        self.wake.port()
    }
}

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        // An empty local connection wakes accept; it has no credential and cannot request focus or a hand-over.
        let _ = TcpStream::connect_timeout(&self.wake, WAIT);
        if let Some(listener) = self.listener.take() {
            let _ = listener.join();
        }
    }
}

/// Asks an existing owner to focus (after taking `hand_over`, when given) before a caller initializes its GUI
/// runtime. `None` means no owner was observed; callers must still acquire the guard before starting a host.
///
/// # Errors
/// Returns a lock-file error. An unanswered request or failed focus-only request is `Some(false)`.
/// A delivered hand-over is `Some(true)` even when the window cannot take focus.
pub fn focus_existing(data: &Path, hand_over: Option<&HandOver>) -> io::Result<Option<bool>> {
    let lock = match OpenOptions::new()
        .read(true)
        .write(true)
        .open(data.join("instance.lock"))
    {
        Ok(lock) => lock,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    match lock.try_lock() {
        Ok(()) => Ok(None),
        // The owner is this app closing for its restart: nothing to focus; admission waits for its lock.
        Err(TryLockError::WouldBlock) if restarting(data) => Ok(None),
        Err(TryLockError::WouldBlock) => Ok(Some(request(data, hand_over))),
        Err(TryLockError::Error(error)) => Err(error),
    }
}

/// Marks the next launch into `data` as this app's own restart. Call it just before the app relaunches itself.
///
/// # Errors
/// Returns the write error; the relaunch then behaves as an ordinary second launch.
pub fn mark_restart(data: &Path) -> io::Result<()> {
    vw_platform::replace_private(&data.join(RESTART_MARK), now_ms().to_string().as_bytes())
        .map_err(io::Error::other)
}

use vw_platform::wall_clock::unix_ms as now_ms;

/// A fresh restart mark is present.
fn restarting(data: &Path) -> bool {
    std::fs::read_to_string(data.join(RESTART_MARK))
        .ok()
        .and_then(|text| text.trim().parse::<u64>().ok())
        .is_some_and(|at| {
            now_ms().saturating_sub(at) <= u64::try_from(RESTART_FRESH.as_millis()).unwrap_or(0)
        })
}

/// Waits up to [`RESTART_WAIT`] for the closing owner's lock; false when it is still held.
fn wait_for_owner_exit(lock: &File) -> io::Result<bool> {
    let deadline = Instant::now() + RESTART_WAIT;
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
        match lock.try_lock() {
            Ok(()) => return Ok(true),
            Err(TryLockError::WouldBlock) => {}
            Err(TryLockError::Error(error)) => return Err(error),
        }
    }
    Ok(false)
}

/// Claims the data folder or asks its existing app to show its window.
/// `focus` returns true only when the window was shown, unminimized and focused; `deliver` receives each admitted
/// hand-over: this launch's own `hand_over` when it becomes the owner, then each later launch's, before `focus`
/// runs. When another app owns the folder, `hand_over` goes to that app instead.
///
/// # Errors
/// Returns filesystem/listener errors; callers must exit without starting a core.
pub fn start(
    data: &Path,
    hand_over: Option<HandOver>,
    focus: impl Fn() -> bool + Send + 'static,
    deliver: impl Fn(HandOver) + Send + 'static,
) -> io::Result<InstanceStart> {
    vw_platform::ensure_private_dir(data).map_err(io::Error::other)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(data.join("instance.lock"))?;
    let endpoint_path = data.join("instance.json");
    match lock.try_lock() {
        Ok(()) => (),
        Err(TryLockError::WouldBlock) if restarting(data) && wait_for_owner_exit(&lock)? => (),
        Err(TryLockError::WouldBlock) => {
            return Ok(InstanceStart::Existing {
                focused: request(data, hand_over.as_ref()),
            });
        }
        Err(TryLockError::Error(error)) => return Err(error),
    }
    // This process owns the folder: a restart mark (used now, or left by a relaunch that never came) is spent.
    let _ = std::fs::remove_file(data.join(RESTART_MARK));
    // A stale endpoint from a crashed owner must not answer a second launch while this one starts.
    let _ = std::fs::remove_file(&endpoint_path);
    let (listener, own) = vw_netguard::listeners::bind_own()?;
    let wake = listener.local_addr()?;
    let mut random = [0_u8; 32];
    getrandom::fill(&mut random).map_err(|error| io::Error::other(error.to_string()))?;
    let endpoint = Endpoint {
        port: wake.port(),
        instance: hex::encode(random),
    };
    vw_platform::replace_private(&endpoint_path, &serde_json::to_vec(&endpoint)?)
        .map_err(io::Error::other)?;
    if let Some(hand_over) = hand_over {
        deliver(hand_over.admit());
    }
    let stop = Arc::new(AtomicBool::new(false));
    let stopping = stop.clone();
    let thread = std::thread::Builder::new()
        .name("window-focus".to_owned())
        .spawn(move || {
            // The port stays one of Vibe Work's own (refused for every browser page) until the listener closes.
            let _own = own;
            while !stopping.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_read_timeout(Some(WAIT));
                        let _ = stream.set_write_timeout(Some(WAIT));
                        if let Some(request) = read_request(&mut stream, &endpoint.instance) {
                            let accepted = match request {
                                Request::Focus => focus(),
                                Request::HandOver(hand_over) => {
                                    deliver(hand_over.admit());
                                    let _ = focus();
                                    true
                                }
                            };
                            let _ = stream.write_all(if accepted { b"1" } else { b"0" });
                        }
                    }
                    Err(_) => break,
                }
            }
        })?;
    Ok(InstanceStart::First(InstanceGuard {
        _lock: lock,
        stop,
        wake,
        listener: Some(thread),
    }))
}

/// What a second launch asks.
enum Request {
    Focus,
    HandOver(HandOver),
}

/// Reads one request: `None` for a wrong token or a malformed or refused body (nothing is delivered and the
/// window is not focused).
fn read_request(stream: &mut TcpStream, instance: &str) -> Option<Request> {
    let mut token = [0_u8; 64];
    stream.read_exact(&mut token).ok()?;
    if token != instance.as_bytes() {
        return None;
    }
    let mut kind = [0_u8; 1];
    stream.read_exact(&mut kind).ok()?;
    match &kind {
        b"F" => Some(Request::Focus),
        b"H" => {
            let mut length = [0_u8; 4];
            stream.read_exact(&mut length).ok()?;
            let length = usize::try_from(u32::from_be_bytes(length)).ok()?;
            if length > MAX_WIRE_BYTES {
                tracing::warn!(length, "a hand-over was refused: too large");
                return None;
            }
            let mut body = vec![0_u8; length];
            stream.read_exact(&mut body).ok()?;
            match HandOver::decode(&body) {
                Ok(hand_over) => Some(Request::HandOver(hand_over)),
                Err(error) => {
                    tracing::warn!(%error, "a hand-over was refused");
                    None
                }
            }
        }
        _ => None,
    }
}

/// Sends the request, waiting for a starting owner's endpoint; false when no owner answered.
fn request(data: &Path, hand_over: Option<&HandOver>) -> bool {
    let deadline = Instant::now() + STARTING;
    loop {
        match send(&data.join("instance.json"), hand_over) {
            Ok(focused) => return focused,
            Err(error) if Instant::now() < deadline && starting(&error) => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(_) => return false,
        }
    }
}

/// The owner holds the lock but has not published (or not yet opened) its endpoint.
fn starting(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused | io::ErrorKind::InvalidData
    )
}

fn send(path: &Path, hand_over: Option<&HandOver>) -> io::Result<bool> {
    let endpoint: Endpoint = serde_json::from_slice(&std::fs::read(path)?)?;
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, endpoint.port));
    let mut stream = TcpStream::connect_timeout(&address, WAIT)?;
    stream.set_read_timeout(Some(WAIT))?;
    stream.set_write_timeout(Some(WAIT))?;
    let mut message = endpoint.instance.into_bytes();
    match hand_over.filter(|hand_over| !hand_over.is_empty()) {
        None => message.push(b'F'),
        Some(hand_over) => {
            let body = hand_over.encode()?;
            let length = u32::try_from(body.len())
                .ok()
                .filter(|&length| length as usize <= MAX_WIRE_BYTES)
                .ok_or_else(|| io::Error::other("hand-over too large"))?;
            message.push(b'H');
            message.extend_from_slice(&length.to_be_bytes());
            message.extend_from_slice(&body);
        }
    }
    stream.write_all(&message)?;
    let mut reply = [0_u8; 1];
    stream.read_exact(&mut reply)?;
    Ok(reply == *b"1")
}
