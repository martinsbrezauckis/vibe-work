//! Scoped native playback silence for voice answers. Engines restore their prior output when the lease drops.
//! The core holds a lease only while capturing an answer; unsupported engines refuse capture.

/// Restores native playback when its last owner releases the lease, including cancellation.
pub struct AudioQuiet(Option<Box<dyn FnOnce() + Send + Sync>>);
impl AudioQuiet {
    /// A lease whose release schedules restoration of the exact prior output state.
    pub fn new(release: impl FnOnce() + Send + Sync + 'static) -> Self {
        Self(Some(Box::new(release)))
    }
}
impl Drop for AudioQuiet {
    fn drop(&mut self) {
        if let Some(release) = self.0.take() {
            release();
        }
    }
}

impl crate::PreviewService {
    /// Silence all engine tabs before opening a permission microphone.
    ///
    /// # Errors
    /// Any engine that cannot guarantee native silence refuses voice capture.
    pub async fn quiet_audio(&self) -> Result<AudioQuiet, crate::PreviewError> {
        self.engine.quiet_audio().await.map_err(Into::into)
    }
}
