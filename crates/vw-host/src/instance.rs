//! One app per data folder, acquired by Tauri before starting the host.
//! Uses a persistent OS-locked file and a loopback focus endpoint; different data folders remain independent.
//! The lock is never unlinked: a crash releases it without letting contenders lock different inodes.

use std::fmt::Write as _;
use std::fs::{File, OpenOptions, TryLockError};
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};

const WAIT: Duration = Duration::from_secs(2);

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

/// Asks an existing owner to focus before a caller initializes its GUI runtime.
/// `None` means no owner was observed; callers must still acquire the guard before starting a host.
///
/// # Errors
/// Returns a lock-file error. A failed focus request is `Some(false)`.
pub fn focus_existing(data: &Path) -> io::Result<Option<bool>> {
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
        Err(TryLockError::WouldBlock) => Ok(Some(
            request_focus(&data.join("instance.json")).unwrap_or(false),
        )),
        Err(TryLockError::Error(error)) => Err(error),
    }
}

/// Claims the data folder or asks its existing app to show its window.
/// `focus` returns true only when the window was shown, unminimized and focused.
///
/// # Errors
/// Returns filesystem/listener errors; callers must exit without starting a core.
pub fn start(data: &Path, focus: impl Fn() -> bool + Send + 'static) -> io::Result<InstanceStart> {
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
                focused: request_focus(&endpoint_path).unwrap_or(false),
            });
        }
        Err(TryLockError::Error(error)) => return Err(error),
    }
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
    let stop = Arc::new(AtomicBool::new(false));
    let stopping = stop.clone();
    let thread = std::thread::Builder::new()
        .name("window-focus".to_owned())
        .spawn(move || {
            while !stopping.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_read_timeout(Some(WAIT));
                        let _ = stream.set_write_timeout(Some(WAIT));
                        let mut request = [0_u8; 64];
                        if stream.read_exact(&mut request).is_ok()
                            && request == endpoint.instance.as_bytes()
                        {
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

fn request_focus(path: &Path) -> io::Result<bool> {
    let endpoint: Endpoint = serde_json::from_slice(&std::fs::read(path)?)?;
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, endpoint.port));
    let mut stream = TcpStream::connect_timeout(&address, WAIT)?;
    stream.set_read_timeout(Some(WAIT))?;
    stream.set_write_timeout(Some(WAIT))?;
    stream.write_all(endpoint.instance.as_bytes())?;
    let mut reply = [0_u8; 1];
    stream.read_exact(&mut reply)?;
    Ok(reply == *b"1")
}
