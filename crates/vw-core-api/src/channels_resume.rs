//! Resuming an existing conversation, including CLI histories imported under a new id.
//! Depends on the channel registry and supervisor; called by `Channels::request_routed`.
use std::sync::{Arc, Mutex};

use super::{ChannelKey, Channels, Connection, RpcError, Value, local_error, locked, to_rpc};
use crate::routing::result_session;

/// Captured by the resume itself, before its caller can be cancelled or a replacement can bind.
pub(crate) struct RestoreBinding {
    pub(crate) id: String,
    pub(crate) key: ChannelKey,
    pub(crate) conn: Connection,
    /// Keep the OS ownership claim until the transport acknowledges close, even after registry removal.
    pub(crate) lease: Option<Arc<crate::session_lease::Lease>>,
}

pub(crate) type RestoreChannel = Arc<Mutex<Option<RestoreBinding>>>;

impl Channels {
    pub(crate) fn track_restore(&self, id: &str) -> RestoreChannel {
        let channel = Arc::new(Mutex::new(None));
        locked(&self.inner)
            .restores
            .insert(id.to_owned(), Arc::downgrade(&channel));
        channel
    }

    /// Detach synchronously, before the guard ends the restoring mark. Cleanup never looks up a
    /// connection by session ID after an await, so a later binding cannot be closed in its place.
    pub(crate) fn finish_restore(
        &self,
        id: &str,
        channel: &RestoreChannel,
        verified: bool,
    ) -> Option<RestoreBinding> {
        let mut inner = locked(&self.inner);
        if inner
            .restores
            .get(id)
            .is_some_and(|owner| owner.ptr_eq(&Arc::downgrade(channel)))
        {
            inner.restores.remove(id);
        }
        let mut bound = locked(channel).take()?;
        let mut closing_id = bound.id.clone();
        if !verified {
            // Follow a hand-off of this exact channel, never another live slot of the same session.
            if let Some(actual) = inner
                .slots
                .iter()
                .find(|(_, slot)| slot.key == bound.key)
                .map(|(id, _)| id.clone())
            {
                inner.slots.remove(&actual);
                inner.close_when_quiet.remove(&actual);
                self.history_loading.stop(&actual);
                closing_id = actual;
            }
        }
        drop(inner);
        if bound.id != id {
            self.end_restore(&bound.id);
        }
        if verified {
            None
        } else {
            // A hand-off may have replaced the OS claim while preserving this channel's key.
            if let Some(lease) = self.leases.release_channel(&closing_id, bound.key) {
                bound.lease = Some(lease);
            }
            Some(bound)
        }
    }

    fn capture_restore(&self, source: &str, bound: RestoreBinding) {
        let owner = locked(&self.inner)
            .restores
            .get(source)
            .and_then(std::sync::Weak::upgrade);
        if let Some(owner) = owner {
            let mut channel = locked(&owner);
            if channel.is_none() {
                if bound.id != source {
                    self.begin_restore(&bound.id);
                }
                *channel = Some(bound);
            }
        }
    }

    /// `session/resume`: on the session's channel, or on a new one bound on success. Uses the
    /// supervisor's resume (it publishes progress while a slow resume runs).
    pub(super) async fn resume(&self, id: &str, params: Value) -> Result<Value, RpcError> {
        let lock = self.session_lock(id);
        let _held = lock.lock().await;
        let method = "session/resume";
        let lease = self
            .leases
            .claim(id)
            .map_err(|error| local_error(method, error))?;
        let existing = {
            let mut inner = locked(&self.inner);
            inner
                .slots
                .get_mut(id)
                .filter(|slot| slot.alive())
                .map(|slot| {
                    slot.last_used = std::time::Instant::now();
                    (slot.conn.clone(), slot.key)
                })
        };
        if let Some((conn, key)) = existing {
            self.capture_restore(
                id,
                RestoreBinding {
                    id: id.to_owned(),
                    key,
                    conn: conn.clone(),
                    lease,
                },
            );
            return self
                .supervisor
                .resume_on(&conn, id, params)
                .await
                .map_err(|e| to_rpc(method, self.wait, e));
        }
        // Captured before the open: Vibe reads the command-server config after this point.
        let applied = self.switch_generation();
        let (conn, key) = self.open(method).await?;
        match self.supervisor.resume_on(&conn, id, params).await {
            Ok(result) => {
                let actual = result_session(&result).unwrap_or(id);
                let lease = if actual == id {
                    lease
                } else {
                    match self.leases.claim(actual) {
                        Ok(lease) => lease,
                        Err(error) => {
                            conn.close_input().await;
                            return Err(local_error(method, error));
                        }
                    }
                };
                self.capture_restore(
                    id,
                    RestoreBinding {
                        id: actual.to_owned(),
                        key,
                        conn: conn.clone(),
                        lease: lease.clone(),
                    },
                );
                self.leases.retain(actual, key, lease);
                self.bind_applied(actual, conn, key, applied);
                if actual != id {
                    locked(&self.inner)
                        .imported
                        .insert(id.to_owned(), actual.to_owned());
                }
                Ok(result)
            }
            Err(error) => {
                conn.close_input().await;
                Err(to_rpc(method, self.wait, error))
            }
        }
    }
}
