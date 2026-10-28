//! Idle-sleep requests behind a small test seam, called by the core's single owning worker.
//! Depends on keepawake; acquisition and drop must happen on the same thread (Windows).

/// An OS request released when dropped. It deliberately need not cross threads.
pub trait IdleSleepHold {}

/// The OS edge, replaceable with a fake without changing core activity policy.
pub trait IdleSleepBackend: Send + Sync {
    /// Acquire idle-sleep inhibition, leaving display sleep, locking and explicit Sleep alone.
    ///
    /// # Errors
    /// The OS did not accept the request. Callers must not retry in a loop.
    fn acquire(&self) -> Result<Box<dyn IdleSleepHold>, SleepRefused>;
}

/// The OS refused an idle-sleep request; no OS error text reaches the UI.
#[derive(Debug, thiserror::Error)]
#[error("idle-sleep request refused")]
pub struct SleepRefused;

/// Production backend. Only the worker that acquired the request may drop it.
pub struct NativeIdleSleep;

impl IdleSleepHold for keepawake::KeepAwake {}

impl IdleSleepBackend for NativeIdleSleep {
    fn acquire(&self) -> Result<Box<dyn IdleSleepHold>, SleepRefused> {
        keepawake::Builder::default()
            .idle(true)
            .display(false)
            .sleep(false)
            .app_name("Vibe Work")
            .app_reverse_domain("com.vibework.app")
            .reason("Vibe is working")
            .create()
            .map(|hold| Box::new(hold) as Box<dyn IdleSleepHold>)
            .map_err(|_| SleepRefused)
    }
}
