//! Reopening a handoff source waits for Vibe's detached backend to release its lease.
//! Uses the shared lease inventory, called by handoff and recovery; never retries a resume.
use super::{CoreError, ErrorCode, Holders, PoisonError, Shared, inventory};
use std::sync::Arc;

impl Holders {
    pub(crate) fn handed_off(&self, id: &str) {
        self.handed_off
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id.to_owned());
    }

    pub(crate) async fn wait_for_handoff(
        &self,
        shared: &Arc<Shared>,
        id: &str,
    ) -> Result<(), CoreError> {
        if !self
            .handed_off
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(id)
        {
            return Ok(());
        }
        let deadline = tokio::time::Instant::now() + shared.config.reopen_timeout;
        loop {
            // Subscribe before reading: a release following the snapshot must wake this waiter.
            let mut updates = self.updates.subscribe();
            inventory::ensure(shared).await?;
            if self.inventory.holder(id).is_none() || super::read_once(shared, id).await?.is_some()
            {
                // Free, or now foreign-owned: ordinary holder checks handle the latter.
                self.handed_off
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .remove(id);
                return Ok(());
            }
            // Vibe 3.1.1 can finish a title/turn after /clear answers. Its own lease is
            // not a foreign holder, but a different channel cannot resume that backend yet.
            if tokio::time::timeout_at(deadline, updates.changed())
                .await
                .is_err()
            {
                return Err(CoreError::new(
                    ErrorCode::Timeout,
                    "The previous chat is still closing. Try again when it finishes.",
                ));
            }
        }
    }
}
