//! Captures everything the code under test logs through `tracing`, for "no secrets in logs"
//! assertions. The subscriber is thread-local (`set_default`), so tests that use it run on a
//! current-thread runtime where every task shares the test thread.

use std::io::Write;
use std::sync::{Arc, Mutex};

use tracing_subscriber::fmt::MakeWriter;

/// Shared in-memory log buffer.
#[derive(Clone, Default)]
pub struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    /// Everything logged so far.
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

impl Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for LogBuffer {
    type Writer = LogBuffer;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// Installs a TRACE-level capturing subscriber for the current thread; keep the guard alive.
pub fn capture() -> (LogBuffer, tracing::subscriber::DefaultGuard) {
    let buffer = LogBuffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer(buffer.clone())
        .finish();
    let guard = tracing::subscriber::set_default(subscriber);
    (buffer, guard)
}
