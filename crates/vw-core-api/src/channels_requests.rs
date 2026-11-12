//! Gated request routing and channel open/close lifecycle.
//! Called by Channels and its resume helpers; uses the existing per-session locks and ownership leases.
//! New subscriptions precede requests, and a rejected newly-created session is discarded as before.
use super::{ChannelKey, Channels, DISCARD_WAIT, Feed, local_error, locked, to_rpc};
use crate::routing::{Route, loads_plugins, may_hand_off, result_session, route};
use serde_json::Value;
use std::time::Duration;
use tracing::{debug, error, warn};
use vw_rpc::{Connection, RpcError};

impl Channels {
    /// Sends one gated request on the channel `route` picks (see the module doc). A session-opening request
    /// gets the session's MCP server first (the core adds it after the gate, which refuses a
    /// caller's own `mcpServers`), and its credential is bound to the session the answer names.
    pub(crate) async fn request(&self, method: &str, mut params: Value) -> Result<Value, RpcError> {
        if loads_plugins(method, &params) {
            self.sweep_plugins()
                .await
                .map_err(|e| local_error(method, e))?;
        }
        let ticket = self
            .agent_tools
            .as_ref()
            .and_then(|tools| tools.inject(method, &mut params));
        let result = self.request_routed(method, params).await;
        if let (Some(tools), Some(ticket)) = (&self.agent_tools, ticket) {
            tools.settle(ticket, result.as_ref().ok());
        }
        result
    }

    pub(super) async fn request_routed(
        &self,
        method: &str,
        params: Value,
    ) -> Result<Value, RpcError> {
        match route(method, &params) {
            Route::Lobby => self.lobby(method).await?.request(method, params).await,
            Route::New => self.on_new_channel(method, params).await,
            Route::Resume(id) => self.resume(&id, params).await,
            Route::Delete(id) => {
                self.close(&id).await;
                self.lobby(method).await?.request(method, params).await
            }
            Route::Session(id) => {
                if method == "turn/interrupt"
                    && let Some(conn) = self.child_control(&id)
                {
                    return conn.request(method, params).await;
                }
                let Some(conn) = self.bound(&id) else {
                    // A subagent's card is answered on the channel it arrived on (its parent's).
                    if let Some(conn) = (method == "callback/result")
                        .then(|| params.pointer("/result/callbackId").and_then(Value::as_str))
                        .flatten()
                        .and_then(|callback| self.guest_channel(callback))
                    {
                        return conn.request(method, params).await;
                    }
                    return self.lobby(method).await?.request(method, params).await;
                };
                let hand_off = may_hand_off(method, &params);
                let result = conn.request(method, params).await?;
                if let Some(new_id) = result_session(&result).filter(|new| hand_off && *new != id)
                    && let Err(error) = self.rebind(&id, new_id)
                {
                    // The channel now carries `new_id`; closing it waits for Vibe to let go of it.
                    self.close(&id).await;
                    // An attaching fork created `new_id` just now: without ownership it is deleted again.
                    // A hand-off (`/clear`, a rewind into a new session) moved the user's existing
                    // conversation to `new_id`, so it is kept.
                    if method == "session/fork" {
                        self.discard_created(method, new_id).await;
                    }
                    return Err(local_error(method, error));
                }
                Ok(result)
            }
        }
    }

    pub(super) async fn lobby(&self, method: &str) -> Result<Connection, RpcError> {
        self.supervisor
            .connection(self.wait)
            .await
            .map_err(|e| to_rpc(method, self.wait, e))
    }

    /// Opens a channel, subscribes the pump to it, and returns it with its key.
    pub(super) async fn open(&self, method: &str) -> Result<(Connection, ChannelKey), RpcError> {
        let opened = self
            .supervisor
            .open_channel(self.wait)
            .await
            .map_err(|e| to_rpc(method, self.wait, e))?;
        let channel_id = opened.conn.channel_id().unwrap_or_default();
        let key = (opened.generation, channel_id);
        let sub = opened.conn.subscribe();
        // The pump outlives every request; a send error means the core is shutting down.
        let _ = self.feeds.send(Feed { key, sub });
        debug!(
            channel_id,
            generation = key.0,
            method,
            "channel opened for a session"
        );
        Ok((opened.conn, key))
    }

    /// `session/start` / `session/continue` on a fresh channel.
    pub(super) async fn on_new_channel(
        &self,
        method: &str,
        params: Value,
    ) -> Result<Value, RpcError> {
        let (conn, key) = self.open(method).await?;
        match conn.request(method, params).await {
            Ok(result) => {
                match result_session(&result) {
                    Some(id) => {
                        let lease = match self.leases.claim(id) {
                            Ok(lease) => lease,
                            Err(error) => {
                                conn.close_input().await;
                                // `session/start` created this session just now: without ownership it
                                // is deleted again, so it cannot stay behind as an ordinary kept chat.
                                // `session/continue` reopened an existing session: it is kept.
                                if method == "session/start" {
                                    if tokio::time::timeout(DISCARD_WAIT, conn.closed())
                                        .await
                                        .is_err()
                                    {
                                        warn!(
                                            channel_id = key.1,
                                            "a closed channel was not acknowledged within 10 s"
                                        );
                                    }
                                    self.discard_created(method, id).await;
                                }
                                return Err(local_error(method, error));
                            }
                        };
                        self.leases.retain(id, lease);
                        self.bind(id, conn, key);
                    }
                    None => conn.close_input().await,
                }
                Ok(result)
            }
            Err(error) => {
                conn.close_input().await;
                Err(error)
            }
        }
    }

    /// Deletes a session that `method` created just now and whose ownership could not be claimed. Its
    /// channel must already be closed (Vibe refuses to delete a session a channel holds). Best effort:
    /// a failure is logged and the caller still returns the ownership error. Vibe stores a session on its
    /// first turn, so "not found" means there was nothing to delete. The core's store never loaded the
    /// session (the request failed), so there is no bookkeeping to finish.
    pub(super) async fn discard_created(&self, method: &str, id: &str) {
        error!(
            session_id = id,
            method, "session ownership could not be claimed for a new session; deleting it again"
        );
        let params = serde_json::json!({"sessionId": id});
        let deleted = tokio::time::timeout(DISCARD_WAIT, async {
            self.lobby("session/delete")
                .await?
                .request("session/delete", params)
                .await
        })
        .await;
        match deleted {
            Ok(Ok(_)) => debug!(session_id = id, "the new session was deleted again"),
            Ok(Err(RpcError::Remote { error, .. }))
                if error.code == vw_protocol::ProtocolErrorCode::NotFound =>
            {
                debug!(session_id = id, "the new session was never stored");
            }
            Ok(Err(error)) => error!(
                session_id = id,
                %error,
                "the new session could not be deleted again; it stays in the list as an ordinary chat"
            ),
            Err(_) => error!(
                session_id = id,
                "the new session was not deleted within 10 s; it may stay in the list as an ordinary chat"
            ),
        }
    }

    /// Closes the session's channel (if any) and waits for the server's acknowledgement, at most
    /// 10 s: the harness flushes and releases the session lease, so the Vibe CLI can take it. Ends any
    /// work the session still had on it (Vibe ends the turn of a closed channel).
    pub(crate) async fn close(&self, id: &str) -> bool {
        self.close_matching(id, None).await
    }

    /// Close only the channel that owned an earlier request. A Stop followed by a new send may
    /// replace it before the old request reports failure; that failure must not close the new turn.
    pub(crate) async fn close_if(&self, id: &str, key: ChannelKey) -> bool {
        self.close_matching(id, Some(key)).await
    }

    pub(super) async fn close_matching(&self, id: &str, expected: Option<ChannelKey>) -> bool {
        let lock = self.session_lock(id);
        let held = lock.lock().await;
        let slot = {
            let mut inner = locked(&self.inner);
            if expected.is_some_and(|key| inner.slots.get(id).is_none_or(|slot| slot.key != key)) {
                None
            } else {
                inner.close_when_quiet.remove(id);
                self.history_loading.stop(id);
                inner.slots.remove(id)
            }
        };
        let Some(slot) = slot else {
            drop(held);
            self.forget_lock(id, &lock);
            return false;
        };
        slot.conn.close_input().await;
        if tokio::time::timeout(Duration::from_secs(10), slot.conn.closed())
            .await
            .is_err()
        {
            warn!(
                channel_id = slot.key.1,
                "a closed channel was not acknowledged within 10 s"
            );
        }
        debug!(channel_id = slot.key.1, "session channel closed");
        self.leases.release(id);
        drop(held);
        self.forget_lock(id, &lock);
        true
    }
}
