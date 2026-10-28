//! Secret text transfer to the operating-system clipboard with a short lifetime.
//!
//! The service owns one worker thread. Clipboard operations and expiration are serialized there,
//! and the clipboard handle stays alive while the service does, which is required by Linux owners.
//! Depends on: the native clipboard API, Tokio for async replies, and zeroizing storage.
//! Called by: core vault copy commands. No UI event carries the copied value.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use tokio::sync::oneshot;
use zeroize::Zeroizing;

/// Maximum time copied text is retained by this service.
pub const SECRET_CLIPBOARD_LIFETIME: Duration = Duration::from_secs(30);
const COPY_WAIT: Duration = Duration::from_secs(5);

/// A clipboard failure without secret content in its representation.
#[derive(Debug, thiserror::Error)]
pub enum ClipboardError {
    /// The platform clipboard could not be opened or written.
    #[error("clipboard operation failed")]
    Operation,
    /// The clipboard worker could not be started or has stopped.
    #[error("clipboard service unavailable")]
    Unavailable,
    /// The platform did not finish the write within the allowed period.
    #[error("clipboard operation timed out")]
    TimedOut,
}

/// Operations supplied by the native clipboard or a test implementation.
///
/// Implementations must not log or persist the text passed to `set_text`.
pub trait ClipboardBackend: Send + 'static {
    /// Replaces the clipboard with sensitive text.
    ///
    /// # Errors
    /// Returns an error if the platform cannot accept the value.
    fn set_text(&mut self, text: &str) -> Result<(), ClipboardError>;

    /// Reads the current text to protect clipboard content replaced by another application.
    ///
    /// # Errors
    /// Returns an error if the current clipboard is inaccessible or is not text.
    fn get_text(&mut self) -> Result<String, ClipboardError>;

    /// Clears the current clipboard.
    ///
    /// # Errors
    /// Returns an error if the platform refuses the clear operation.
    fn clear(&mut self) -> Result<(), ClipboardError>;
}

enum Request {
    Copy(
        Zeroizing<String>,
        bool,
        Arc<AtomicBool>,
        oneshot::Sender<Result<(), ClipboardError>>,
    ),
    Shutdown,
}

/// Owns native clipboard access and expires copied secrets.
pub struct ClipboardService {
    sender: mpsc::Sender<Request>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl ClipboardService {
    /// Starts a service whose native clipboard is opened only on the first copy.
    ///
    /// # Errors
    /// Returns an error if the worker thread cannot be started.
    pub fn new() -> Result<Self, ClipboardError> {
        Self::with_backend(
            Box::new(NativeClipboard::default()),
            SECRET_CLIPBOARD_LIFETIME,
        )
    }

    /// Starts a service with a supplied clipboard and lifetime.
    ///
    /// A fake backend and short duration allow tests without a desktop session.
    ///
    /// # Errors
    /// Returns an error if the worker thread cannot be started.
    pub fn with_backend(
        backend: Box<dyn ClipboardBackend>,
        lifetime: Duration,
    ) -> Result<Self, ClipboardError> {
        let (sender, receiver) = mpsc::channel();
        let worker = std::thread::Builder::new()
            .name("secret-clipboard".into())
            .spawn(move || run_worker(backend, &receiver, lifetime))
            .map_err(|_| ClipboardError::Unavailable)?;
        Ok(Self {
            sender,
            worker: Mutex::new(Some(worker)),
        })
    }

    /// Copies sensitive text and starts a fresh expiry period after the write succeeds.
    ///
    /// The value is never sent to a window or event stream. Failed requests are dropped and
    /// zeroized by the worker.
    ///
    /// # Errors
    /// Returns an error if the clipboard is unavailable or the worker has shut down.
    pub async fn copy(&self, value: Zeroizing<String>) -> Result<(), ClipboardError> {
        self.copy_with_expiry(value, true).await
    }

    /// Copies ordinary owner text without the vault's expiry timer, including when no web gesture is present.
    ///
    /// # Errors
    /// Returns an error if the clipboard is unavailable or the worker has shut down.
    pub async fn write_text(&self, value: String) -> Result<(), ClipboardError> {
        self.copy_with_expiry(Zeroizing::new(value), false).await
    }

    async fn copy_with_expiry(
        &self,
        value: Zeroizing<String>,
        expires: bool,
    ) -> Result<(), ClipboardError> {
        let (reply, response) = oneshot::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        self.sender
            .send(Request::Copy(value, expires, Arc::clone(&cancelled), reply))
            .map_err(|_| ClipboardError::Unavailable)?;
        match tokio::time::timeout(COPY_WAIT, response).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(ClipboardError::Unavailable),
            Err(_) => {
                cancelled.store(true, Ordering::Release);
                Err(ClipboardError::TimedOut)
            }
        }
    }

    /// Stops the worker and discards its pending value.
    ///
    /// The clipboard is cleared on shutdown only if it still contains this service's last value.
    pub async fn shutdown(&self) {
        let _ = self.sender.send(Request::Shutdown);
        let worker = self.worker.lock().ok().and_then(|mut slot| slot.take());
        if let Some(worker) = worker {
            let _ = tokio::task::spawn_blocking(move || worker.join()).await;
        }
    }
}

impl Drop for ClipboardService {
    fn drop(&mut self) {
        let _ = self.sender.send(Request::Shutdown);
        if let Ok(mut worker) = self.worker.lock()
            && let Some(worker) = worker.take()
        {
            let _ = worker.join();
        }
    }
}

fn run_worker(
    mut backend: Box<dyn ClipboardBackend>,
    receiver: &mpsc::Receiver<Request>,
    lifetime: Duration,
) {
    let mut current: Option<(Zeroizing<String>, Instant)> = None;
    loop {
        let request = if let Some((_, deadline)) = &current {
            match receiver.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(request) => Some(request),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    clear_if_current(backend.as_mut(), &mut current);
                    continue;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => None,
            }
        } else {
            receiver.recv().ok()
        };
        match request {
            Some(Request::Copy(value, expires, cancelled, reply)) => {
                if cancelled.load(Ordering::Acquire) || reply.is_closed() {
                    let _ = reply.send(Err(ClipboardError::TimedOut));
                    continue;
                }
                let result = backend.set_text(&value);
                let wrote = result.is_ok();
                if result.is_ok() {
                    // An old copy must never expire after a newer one has been accepted.
                    current = Some((value, Instant::now() + lifetime));
                }
                if wrote && (cancelled.load(Ordering::Acquire) || reply.is_closed()) {
                    clear_if_current(backend.as_mut(), &mut current);
                }
                if reply.send(result).is_err() && wrote {
                    clear_if_current(backend.as_mut(), &mut current);
                }
                if wrote && !expires {
                    current = None;
                }
            }
            Some(Request::Shutdown) | None => {
                clear_if_current(backend.as_mut(), &mut current);
                break;
            }
        }
    }
}

fn clear_if_current(
    backend: &mut dyn ClipboardBackend,
    current: &mut Option<(Zeroizing<String>, Instant)>,
) {
    if let Some((expected, _)) = current.take()
        && backend
            .get_text()
            .map(Zeroizing::new)
            .is_ok_and(|actual| actual.as_str() == expected.as_str())
    {
        let _ = backend.clear();
    }
}

#[derive(Default)]
struct NativeClipboard {
    clipboard: Option<arboard::Clipboard>,
}

impl NativeClipboard {
    fn clipboard(&mut self) -> Result<&mut arboard::Clipboard, ClipboardError> {
        if self.clipboard.is_none() {
            self.clipboard =
                Some(arboard::Clipboard::new().map_err(|_| ClipboardError::Operation)?);
        }
        self.clipboard.as_mut().ok_or(ClipboardError::Operation)
    }
}

impl ClipboardBackend for NativeClipboard {
    fn set_text(&mut self, text: &str) -> Result<(), ClipboardError> {
        let clipboard = self.clipboard()?;
        #[cfg(target_os = "linux")]
        {
            use arboard::SetExtLinux as _;
            clipboard
                .set()
                .exclude_from_history()
                .text(text)
                .map_err(|_| ClipboardError::Operation)
        }
        #[cfg(target_os = "macos")]
        {
            use arboard::SetExtApple as _;
            clipboard
                .set()
                .exclude_from_history()
                .text(text)
                .map_err(|_| ClipboardError::Operation)?;
            if let Err(error) = crate::clipboard_mac::mark_transient() {
                if clipboard
                    .get_text()
                    .map(Zeroizing::new)
                    .is_ok_and(|read| read.as_str() == text)
                {
                    let _ = clipboard.clear();
                }
                return Err(error);
            }
            Ok(())
        }
        #[cfg(windows)]
        {
            use arboard::SetExtWindows as _;
            clipboard
                .set()
                .exclude_from_history()
                .exclude_from_cloud()
                .exclude_from_monitoring()
                .text(text)
                .map_err(|_| ClipboardError::Operation)
        }
    }

    fn get_text(&mut self) -> Result<String, ClipboardError> {
        self.clipboard()?
            .get_text()
            .map_err(|_| ClipboardError::Operation)
    }

    fn clear(&mut self) -> Result<(), ClipboardError> {
        self.clipboard()?
            .clear()
            .map_err(|_| ClipboardError::Operation)
    }
}
