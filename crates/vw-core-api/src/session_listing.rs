//! Session listing and completion signal for startup work. Depends on the gate, store and name cache;
//! called by the dispatcher. The scanner is released only after the last listing page has loaded.

use crate::{CommandResult, Core, CoreError, Origin, SessionRowView};
use serde_json::{Map, Value};

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
        let items = reply
            .result
            .get("items")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        self.shared.names.note_listing(&items);
        crate::plan_profile::note_listing(&self.shared, &items);
        // sessions seen for the first time are filed by their folder.
        crate::projects_filing::listed(&self.shared, &items).await;
        let rows = self.shared.store.reconcile_listing(&items);
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
        Ok(CommandResult::Sessions {
            rows: rows
                .into_iter()
                .map(|row| {
                    let mut row = SessionRowView::from(row);
                    row.access_mode = row.session["id"].as_str().and_then(|id| {
                        crate::restore::display_mode(&self.shared, id, &row.session)
                    });
                    row
                })
                .collect(),
            next_cursor: text("nextCursor"),
            continue_session_id: text("continueSessionId"),
        })
    }
}
