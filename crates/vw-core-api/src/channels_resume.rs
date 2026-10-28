//! Resuming an existing conversation, including CLI histories imported under a new id.
//! Depends on the channel registry and supervisor; called by `Channels::request_routed`.
use super::{Channels, RpcError, Value, local_error, locked, result_session, to_rpc};

impl Channels {
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
        if let Some(conn) = self.bound(id) {
            return self
                .supervisor
                .resume_on(&conn, id, params)
                .await
                .map_err(|e| to_rpc(method, self.wait, e));
        }
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
                self.leases.retain(actual, lease);
                self.bind(actual, conn, key);
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
