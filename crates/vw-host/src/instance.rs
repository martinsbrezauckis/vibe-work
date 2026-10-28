//! One app per data folder, acquired by Tauri before starting the host.
//! Uses a persistent OS-locked file and a loopback focus endpoint; different data folders remain independent.
//! The lock is never unlinked: a crash releases it without letting contenders lock different inodes.
//! A second launch may carry a hand-over ("Send to Vibe Work", [`crate::hand_over`]): it is written after the
//! endpoint's token, the running app admits it again and gives it to its `deliver` seam, then focuses.
//! Request: the 64-character token, then `F` (focus) or `H` + a 4-byte big-endian length + the hand-over's JSON.
//! Reply: one byte, `1` when the window was focused.

use std::fmt::Write as _;
use std::fs::{File, OpenOptions, TryLockError};
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
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

#[derive(Serialize, Deserialize)]
struct Endpoint {
    port: u16,
    instance: String,
}

/// The outcome of launching into a data folder.
pub enum InstanceStart {
    /// This process owns the folder until the guard is dropped.
    First(InstanceGuard),
    /// Another process owns it; whether its window accepted the focus request.
    Existing {
        /// Whether the running window accepted focus.
        focused: bool,
    },
}

/// Keeps the folder lock and focus listener alive for the entire app lifetime.
pub struct InstanceGuard {
    _lock: File,
    stop: Arc<AtomicBool>,
    listener: Option<std::thread::JoinHandle<()>>,
}

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(listener) = self.listener.take() {
            let _ = listener.join();
        }
    }
}

/// Asks an existing owner to focus (after taking `hand_over`, when given) before a caller initializes its GUI
/// runtime. `None` means no owner was observed; callers must still acquire the guard before starting a host.
///
/// # Errors
/// Returns a lock-file error. A failed focus request is `Some(false)`.
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
        Err(TryLockError::WouldBlock) => Ok(Some(request(data, hand_over))),
        Err(TryLockError::Error(error)) => Err(error),
    }
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
        Err(TryLockError::WouldBlock) => {
            return Ok(InstanceStart::Existing {
                focused: request(data, hand_over.as_ref()),
            });
        }
        Err(TryLockError::Error(error)) => return Err(error),
    }
    // A stale endpoint from a crashed owner must not answer a second launch while this one starts.
    let _ = std::fs::remove_file(&endpoint_path);
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    listener.set_nonblocking(true)?;
    let mut random = [0_u8; 32];
    getrandom::fill(&mut random).map_err(|error| io::Error::other(error.to_string()))?;
    let endpoint = Endpoint {
        port: listener.local_addr()?.port(),
        instance: random
            .iter()
            .fold(String::with_capacity(64), |mut text, byte| {
                let _ = write!(text, "{byte:02x}");
                text
            }),
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
            while !stopping.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_nonblocking(false);
                        let _ = stream.set_read_timeout(Some(WAIT));
                        let _ = stream.set_write_timeout(Some(WAIT));
                        if let Some(request) = read_request(&mut stream, &endpoint.instance) {
                            if let Request::HandOver(hand_over) = request {
                                deliver(hand_over.admit());
                            }
                            let _ = stream.write_all(if focus() { b"1" } else { b"0" });
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(25));
                    }
                    Err(_) => break,
                }
            }
        })?;
    Ok(InstanceStart::First(InstanceGuard {
        _lock: lock,
        stop,
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
