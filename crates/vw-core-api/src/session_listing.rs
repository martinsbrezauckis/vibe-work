//! Session listing and completion signal for startup work. Depends on the gate, store and name cache;
//! called by the dispatcher. The scanner is released only after the last listing page has loaded.
//! Invariant: the chat `vibe --continue` resumes is named only when the Vibe CLI's session folder holds it.

use crate::{CommandResult, Core, CoreError, ErrorCode, Origin, SessionRowView};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// Empty child listings for inactive, unchanged parent rows, scoped to a server generation.
/// Parents with children are always refreshed, including when all their children are archived.
#[derive(Default)]
pub(crate) struct ChildListings {
    empty: tokio::sync::Mutex<BTreeMap<String, (u64, Value)>>,
}

impl Core {
    /// `session/list` through the gate, merged with local-only sessions.
    pub(crate) async fn list_sessions(
        &self,
        origin: Origin,
        limit: Option<u32>,
        cursor: Option<String>,
        include_archived: Option<bool>,
    ) -> Result<CommandResult, CoreError> {
        let mut params = Map::new();
        if let Some(include) = include_archived {
            params.insert("includeArchived".to_owned(), Value::Bool(include));
        }
        if let Some(limit) = limit {
            params.insert("limit".to_owned(), Value::from(limit));
        }
        if let Some(cursor) = cursor {
            params.insert("cursor".to_owned(), Value::String(cursor));
        }
        let params = Value::Object(params);
        let reply = self.forward(origin, "session/list", params, None).await?;
        let mut items = reply
            .result
            .get("items")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        // Vibe's default page contains roots only. Child sessions need explicit parent queries; reading them
        // from the lobby preserves their leases and gives the sidebar its stored descendants after a restart.
        items.extend(
            self.listed_children(origin, &items, include_archived)
                .await?,
        );
        self.shared.names.note_listing(&items);
        crate::plan_profile::note_listing(&self.shared, &items);
        // sessions seen for the first time are filed by their folder.
        crate::projects_filing::listed(&self.shared, &items).await;
        let rows = self.shared.store.reconcile_listing(&items);
        let mut views = Vec::with_capacity(rows.len());
        for row in rows {
            let mut row = SessionRowView::from(row);
            row.access_mode = row.session["id"]
                .as_str()
                .and_then(|id| crate::restore::display_mode(&self.shared, id, &row.session));
            row.owned_by_parent = Some(
                crate::session_ownership::owned_by_parent(&self.shared, &row.session)
                    .await
                    .unwrap_or_else(|error| {
                        tracing::warn!(session_id = ?row.session["id"], %error, "could not read parent ownership for chat listing");
                        false
                    }),
            );
            views.push(row);
        }
        let text = |key: &str| {
            reply
                .result
                .get(key)
                .and_then(Value::as_str)
                .map(str::to_owned)
        };
        if reply.result.get("nextCursor").is_none_or(Value::is_null) {
            self.shared.secret_scan.loaded();
        }
        let continue_session_id = match (
            text("continueSessionId"),
            self.shared.config.cli_session_dir.clone(),
        ) {
            (Some(id), Some(cli)) => {
                crate::pages::blocking(move || cli_holds(&cli, &id).then_some(id))
                    .await
                    .unwrap_or(None)
            }
            _ => None,
        };
        Ok(CommandResult::Sessions {
            rows: views,
            next_cursor: text("nextCursor"),
            continue_session_id,
        })
    }
}

/// True when the Vibe CLI's session folder `cli` holds the chat `id`: Vibe keeps each chat's store in
/// `<save dir>/unified/<id>` (Vibe 3.0.3 `app_server/_subagent_runs.py:223-227`). Vibe names its newest chat even when
/// it saved nothing there (session logging off keeps chats in a throwaway folder), so the folder is checked. An id
/// that is not one plain name is never joined to the path.
fn cli_holds(cli: &std::path::Path, id: &str) -> bool {
    let plain = !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    plain && cli.join("unified").join(id).is_dir()
}

impl Core {
    /// Fetches descendants without repeatedly sweeping the catalog for unchanged childless parents.
    async fn listed_children(
        &self,
        origin: Origin,
        roots: &[Value],
        include_archived: Option<bool>,
    ) -> Result<Vec<Value>, CoreError> {
        let generation = match &*self.shared.supervisor.state().borrow() {
            vw_supervisor::SupervisorState::Ready { generation, .. } => *generation,
            _ => 0,
        };
        // Single flight: overlapping sidebar refreshes share the completed discoveries. Never cache a
        // running/blocked parent: spawning happens in a turn, which advances the parent's public row.
        let mut empty = self.shared.child_listings.empty.lock().await;
        empty.retain(|_, (at, _)| *at == generation);
        let mut parents: VecDeque<Value> = roots.iter().cloned().collect();
        let mut seen: BTreeSet<String> = roots
            .iter()
            .filter_map(|row| row["id"].as_str().map(str::to_owned))
            .collect();
        let mut pending = tokio::task::JoinSet::new();
        let mut children = Vec::new();
        loop {
            while pending.len() < 8 {
                let Some(parent) = parents.pop_front() else {
                    break;
                };
                let Some(id) = parent["id"].as_str() else {
                    continue;
                };
                if empty.get(id).is_some_and(|(_, row)| row == &parent) {
                    continue;
                }
                let core = self.clone();
                pending.spawn(async move {
                    let rows = core
                        .direct_children(origin, parent["id"].as_str().unwrap_or_default())
                        .await;
                    (parent, rows)
                });
            }
            let Some(result) = pending.join_next().await else {
                break;
            };
            let (parent, rows) = result.map_err(|error| {
                CoreError::new(
                    ErrorCode::Internal,
                    format!("could not list child sessions: {error}"),
                )
            })?;
            let rows = rows?;
            let id = parent["id"].as_str().unwrap_or_default().to_owned();
            if rows.is_empty()
                && matches!(
                    parent["status"]["type"].as_str(),
                    Some("idle" | "archived" | "failed")
                )
            {
                empty.insert(id, (generation, parent));
            } else {
                empty.remove(&id);
            }
            for child in rows.into_iter().filter(|row| {
                include_archived == Some(true) || row.get("archivedAt").is_none_or(Value::is_null)
            }) {
                if let Some(id) = child["id"].as_str()
                    && seen.insert(id.to_owned())
                {
                    parents.push_back(child.clone());
                    children.push(child);
                }
            }
        }
        children.sort_by(|a, b| {
            b["updatedAt"]
                .as_i64()
                .cmp(&a["updatedAt"].as_i64())
                .then_with(|| a["id"].as_str().cmp(&b["id"].as_str()))
        });
        Ok(children)
    }

    async fn direct_children(&self, origin: Origin, parent: &str) -> Result<Vec<Value>, CoreError> {
        let mut children = Vec::new();
        let mut cursor: Option<String> = None;
        let mut seen = BTreeSet::new();
        loop {
            // Discover archived children too: unarchiving a child need not change its parent's row.
            let mut params = json!({"parentSessionId":parent,"limit":500,"includeArchived":true});
            if let Some(cursor) = &cursor {
                params["cursor"] = Value::String(cursor.clone());
            }
            let reply = self.forward(origin, "session/list", params, None).await?;
            children.extend(
                reply.result["items"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|row| row["parentSessionId"] == parent)
                    .cloned(),
            );
            let Some(next) = reply.result["nextCursor"].as_str() else {
                break;
            };
            if !seen.insert(next.to_owned()) {
                return Err(CoreError::new(
                    ErrorCode::ServerError,
                    "Vibe repeated a child-session listing cursor",
                ));
            }
            cursor = Some(next.to_owned());
        }
        Ok(children)
    }
}
