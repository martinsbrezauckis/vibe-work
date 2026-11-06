//! Distinguishes subagent ownership from fork ancestry using Vibe's parent childSessions records.
//! Uses lobby reads (no leases); called by listing and delete admission.
use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

use crate::core::Shared;
use crate::{CoreError, ErrorCode};

const REFUSAL: &str = "Subagent chats are removed when you delete their parent chat.";

async fn read(shared: &Shared, id: &str) -> Result<Value, CoreError> {
    shared
        .gate
        .call(
            "session/read",
            json!({"sessionId":id,"history":{"limit":1}}),
            None,
        )
        .await
        .map(|reply| reply.result["state"].clone())
        .map_err(|error| CoreError::from_gate("session/read", &error))
}
fn child_ids(state: &Value) -> BTreeSet<String> {
    state["childSessions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| row["id"].as_str().map(str::to_owned))
        .collect()
}

/// Facts live for one listing request only. Delete/Stop always create a fresh reader.
#[derive(Default)]
pub(crate) struct OwnershipReads {
    parents: BTreeMap<String, Result<Option<BTreeSet<String>>, CoreError>>,
    catalogs: BTreeMap<(String, Option<String>), Result<BTreeSet<String>, CoreError>>,
}

impl OwnershipReads {
    /// Descendant traversal already read every page, including archived children.
    pub(crate) fn note_children(&mut self, parent: &str, rows: &[Value]) {
        self.catalogs.insert(
            ("parentSessionId".into(), Some(parent.into())),
            Ok(rows
                .iter()
                .filter_map(|row| row["id"].as_str().map(str::to_owned))
                .collect()),
        );
    }

    async fn parent(
        &mut self,
        shared: &Shared,
        id: &str,
    ) -> Result<Option<&BTreeSet<String>>, CoreError> {
        if !self.parents.contains_key(id) {
            let facts = match read(shared, id).await {
                Ok(state) => {
                    let mut children = child_ids(&state);
                    children.extend(shared.store.child_ids(id));
                    Ok(Some(children))
                }
                Err(error) if error.server_code.as_deref() == Some("not_found") => Ok(None),
                Err(error) => Err(error),
            };
            self.parents.insert(id.to_owned(), facts);
        }
        match &self.parents[id] {
            Ok(children) => Ok(children.as_ref()),
            Err(error) => Err(error.clone()),
        }
    }

    async fn catalog_contains(
        &mut self,
        shared: &Shared,
        id: &str,
        key: &str,
        value: Option<&str>,
    ) -> Result<bool, CoreError> {
        let cache_key = (key.to_owned(), value.map(str::to_owned));
        if !self.catalogs.contains_key(&cache_key) {
            let rows = catalog(shared, key, value).await;
            self.catalogs.insert(cache_key.clone(), rows);
        }
        self.catalogs[&cache_key]
            .as_ref()
            .map(|ids| ids.contains(id))
            .map_err(Clone::clone)
    }

    pub(crate) async fn owned(
        &mut self,
        shared: &Shared,
        session: &Value,
    ) -> Result<bool, CoreError> {
        let (Some(parent), Some(id)) =
            (session["parentSessionId"].as_str(), session["id"].as_str())
        else {
            return Ok(false);
        };
        let Some(children) = self.parent(shared, parent).await? else {
            return Ok(false);
        };
        if children.contains(id) {
            return Ok(true);
        }
        // Ancestry alone does not establish ownership. Prove durable child membership before using
        // absence from the root catalog, which omits subagent identities but includes forks.
        if !self
            .catalog_contains(shared, id, "parentSessionId", Some(parent))
            .await?
        {
            return Ok(false);
        }
        Ok(!self
            .catalog_contains(
                shared,
                id,
                "rootSessionId",
                session["rootSessionId"].as_str(),
            )
            .await?)
    }
}

/// Reads every page once; repeated cursors remain errors, including in archived catalogs.
async fn catalog(
    shared: &Shared,
    key: &str,
    value: Option<&str>,
) -> Result<BTreeSet<String>, CoreError> {
    let mut params = json!({"limit":500,"includeArchived":true});
    if let Some(value) = value {
        params[key] = Value::String(value.to_owned());
    }
    let mut cursors = BTreeSet::new();
    let mut ids = BTreeSet::new();
    loop {
        let reply = shared
            .gate
            .call("session/list", params.clone(), None)
            .await
            .map_err(|error| CoreError::from_gate("session/list", &error))?;
        ids.extend(
            reply.result["items"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|row| row["id"].as_str().map(str::to_owned)),
        );
        let Some(next) = reply.result["nextCursor"].as_str() else {
            return Ok(ids);
        };
        if !cursors.insert(next.to_owned()) {
            return Err(CoreError::new(
                ErrorCode::ServerError,
                "Vibe repeated a session-listing cursor",
            ));
        }
        params["cursor"] = Value::String(next.to_owned());
    }
}

/// Fresh ownership admission, independent of any earlier sidebar request.
pub(crate) fn owned_by_parent<'a>(
    shared: &'a Shared,
    session: &'a Value,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<bool, CoreError>> + Send + 'a>> {
    Box::pin(async move { OwnershipReads::default().owned(shared, session).await })
}
/// Keeps the shared dispatch future bounded while checking session-end admission.
pub(crate) fn check_delete<'a>(
    shared: &'a Shared,
    method: &'a str,
    params: &'a Value,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), CoreError>> + Send + 'a>> {
    Box::pin(check_delete_inner(shared, method, params))
}
/// Refuses only a still-owned child, for UI and raw Debug API deletes alike.
async fn check_delete_inner(
    shared: &Shared,
    method: &str,
    params: &Value,
) -> Result<(), CoreError> {
    crate::session_end::check_archive(shared, method, params)?;
    if method != "session/delete" {
        return Ok(());
    }
    let Some(id) = params["sessionId"].as_str() else {
        return Ok(());
    };
    let state = match read(shared, id).await {
        Ok(state) => state,
        // Damaged stores still need the existing confirmed deletion repair path. An
        // unreadable target provides no ownership evidence; the gate and no-follow cleanup still
        // enforce exact consent. This also preserves key-free startup cleanup of temporary chats.
        Err(error)
            if matches!(
                error.server_code.as_deref(),
                Some("not_found" | "internal_error")
            ) =>
        {
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    if owned_by_parent(shared, &state["session"]).await? {
        return Err(CoreError::new(ErrorCode::Denied, REFUSAL).with_method(method));
    }
    Ok(())
}
