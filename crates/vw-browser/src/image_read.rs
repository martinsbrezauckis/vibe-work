//! Mount-bound reads and captures; the driver pins admission and checks identity before releasing page data.
//! Includes a private debug pause to reproduce navigation at the text/image boundary.
#[cfg(feature = "e2e-script")]
use std::sync::{Arc, RwLock};
#[cfg(feature = "e2e-script")]
use tokio::sync::Notify;

#[cfg(feature = "e2e-script")]
pub(crate) type ImageGate = Arc<RwLock<Option<(Arc<Notify>, Arc<Notify>)>>>;

#[cfg(feature = "e2e-script")]
impl crate::Browser {
    /// Pauses one read after its observation and before its image, for navigation race fixtures.
    #[must_use]
    pub fn pause_before_image(&self) -> (Arc<Notify>, Arc<Notify>) {
        let gates = (Arc::new(Notify::new()), Arc::new(Notify::new()));
        if let Ok(mut slot) = self.image_gate.write() {
            *slot = Some(gates.clone());
        }
        gates
    }

    pub(crate) async fn before_image(&self) {
        let gates = self
            .image_gate
            .write()
            .ok()
            .and_then(|mut slot| slot.take());
        if let Some((reached, resume)) = gates {
            reached.notify_one();
            resume.notified().await;
        }
    }
}

impl crate::Browser {
    /// Pin one read/capture to a mount whose page class cannot change during its lifetime.
    pub(crate) async fn bind_read(&self, session: &str, by: crate::Actor) -> Result<Self, String> {
        self.page_of(session).await?;
        let mount = self
            .service
            .page_mount(session)
            .ok_or(crate::refs::PAGE_CLOSED)?;
        if by == crate::Actor::Agent && self.local_network_blocked(session) {
            return Err(crate::LOCAL_NETWORK.into());
        }
        let mut bound = self.clone();
        bound.read_mount = Some(mount);
        bound.read_actor = Some(by);
        bound.check_read_mount()?;
        Ok(bound)
    }

    pub(crate) fn check_read_mount(&self) -> Result<(), String> {
        if self.read_actor == Some(crate::Actor::Agent)
            && self
                .read_mount
                .as_ref()
                .is_some_and(|mount| self.local_network_blocked(&mount.session_id))
        {
            return Err(crate::LOCAL_NETWORK.into());
        }
        if let Some(mount) = &self.read_mount
            && self.service.page_mount(&mount.session_id).as_ref() != Some(mount)
        {
            return Err(crate::refs::PAGE_CLOSED.into());
        }
        Ok(())
    }
}

impl crate::Browser {
    /// The page accessor for a capture's page scripts.
    pub(crate) async fn page_of(&self, session_id: &str) -> Result<String, String> {
        self.check_read_mount()?;
        let page = self.page(session_id).await?;
        self.check_read_mount()?;
        Ok(page)
    }

    /// Runs a capture's page script (same retries as the agent's reads).
    pub(crate) async fn run_script<T: serde::de::DeserializeOwned>(
        &self,
        session_id: &str,
        script: String,
    ) -> Result<T, String> {
        self.run(session_id, script).await
    }

    /// The answer of a script built for `page` that reports `url` may be used (the document-bound authorization rule).
    pub(crate) fn check_answer(
        &self,
        session_id: &str,
        page: &str,
        url: &str,
    ) -> Result<(), String> {
        self.check_read_mount()?;
        self.service.reading_allowed(session_id, page, url)
    }
}
