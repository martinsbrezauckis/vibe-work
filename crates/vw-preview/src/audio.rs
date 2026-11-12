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
    /// Read/set native audio for a live mount, serializing with page replacement.
    /// # Errors
    /// Unsupported engine, a replaced mount or a native failure.
    pub async fn tab_audio(
        &self,
        id: &crate::MountId,
        muted: Option<bool>,
    ) -> Result<serde_json::Value, crate::PreviewError> {
        let _held = self.session_lock(&id.session_id).await;
        if self.page_mount(&id.session_id).as_ref() != Some(id) {
            return Err(crate::EngineError::NotMounted.into());
        }
        self.engine.tab_audio(id, muted).await.map_err(Into::into)
    }

    /// Silence all engine tabs before opening a permission microphone.
    ///
    /// # Errors
    /// Any engine that cannot guarantee native silence refuses voice capture.
    pub async fn quiet_audio(&self) -> Result<AudioQuiet, crate::PreviewError> {
        self.engine.quiet_audio().await.map_err(Into::into)
    }
}
