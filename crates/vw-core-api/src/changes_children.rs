//! Child-session sources and refresh dependencies for non-git Changes.
//! Reads childSessions from the store, loading missing child snapshots through the gated read path.
//! Called by changes and `changes_ops`; visited ids prevent cycles and shared descendants from duplicating effects.

use std::collections::HashSet;

use serde_json::Value;
use vw_session::SessionStore;

use crate::changes_facts::{Facts, facts};
use crate::changes_view::ChangesSubagent;
use crate::core::Shared;

struct Child {
    source: ChangesSubagent,
    updated_at: Option<i64>,
}

fn children(state: &Value) -> Vec<Child> {
    state
        .get("childSessions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|row| {
            let id = row.get("id").or_else(|| row.get("sessionId"))?.as_str()?;
            if id.is_empty() {
                return None;
            }
            Some(Child {
                source: ChangesSubagent {
                    session_id: id.to_owned(),
                    name: row
                        .get("name")
                        .and_then(Value::as_str)
                        .filter(|name| !name.trim().is_empty())
                        .unwrap_or("Subagent")
                        .to_owned(),
                },
                updated_at: row.get("updatedAt").and_then(Value::as_i64),
            })
        })
        .collect()
}

/// Adds each descendant's own facts without merging its turn ids into the shown session.
pub(crate) async fn include_children(shared: &Shared, id: &str, session: &mut Facts) {
    let Some(parent) = shared.store.view(id) else {
        return;
    };
    let mut pending = children(&parent.state);
    pending.reverse();
    let mut visited = HashSet::from([id.to_owned()]);
    while let Some(child) = pending.pop() {
        let updated_at = child.updated_at;
        let child = child.source;
        if !visited.insert(child.session_id.clone()) {
            continue;
        }
        let stale = shared.store.view(&child.session_id).is_none_or(|view| {
            updated_at.is_some_and(|newer| {
                view.state
                    .pointer("/session/updatedAt")
                    .and_then(Value::as_i64)
                    .is_none_or(|loaded| newer > loaded)
            })
        });
        if stale {
            // Native child rows advance on the parent channel without streaming every child effect.
            // Reuse snapshots only while they cover the parent's latest child timestamp.
            crate::pump::refresh(shared, &child.session_id).await;
        }
        let Some(view) = shared.store.view(&child.session_id) else {
            continue;
        };
        let mut descendants = children(&view.state);
        descendants.reverse();
        pending.extend(descendants);
        session.subagents.push((child, facts(&view.state)));
    }
}

/// Every loaded ancestor of id, using only childSessions membership, never a shared folder or agentType.
pub(crate) fn ancestors(store: &SessionStore, id: &str) -> Vec<String> {
    let views = store.views();
    let mut visited = HashSet::from([id.to_owned()]);
    let mut pending = vec![id.to_owned()];
    let mut parents = Vec::new();
    while let Some(child_id) = pending.pop() {
        for view in &views {
            if !visited.contains(&view.session_id)
                && children(&view.state)
                    .iter()
                    .any(|child| child.source.session_id == child_id)
            {
                visited.insert(view.session_id.clone());
                parents.push(view.session_id.clone());
                pending.push(view.session_id.clone());
            }
        }
    }
    parents
}
