//! Saved proposal reviews and records, separate from Vibe's server-owned history.
//! Uses private atomic files; the core hub overlays these entries in session snapshots.
//! Only reviews are persisted: pending captures can never be reconstructed after restart.
use crate::{ProtectedChangeStatus, ProtectedChangeView};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Saved {
    pub(super) view: ProtectedChangeView,
    pub(super) record: Option<vw_proposals::Record>,
}
#[derive(Default)]
pub(super) struct Reviews {
    pub(super) entries: BTreeMap<String, Saved>,
    directory: Option<PathBuf>,
}
impl Reviews {
    pub(super) fn open(directory: Option<PathBuf>) -> Self {
        let mut reviews = Self {
            directory,
            ..Self::default()
        };
        if let Some(directory) = &reviews.directory {
            match std::fs::read_dir(directory) {
                Ok(entries) => {
                    for entry in entries
                        .flatten()
                        .filter(|e| e.path().extension().is_some_and(|e| e == "json"))
                    {
                        let saved = std::fs::read(entry.path())
                            .ok()
                            .and_then(|b| serde_json::from_slice::<Saved>(&b).ok());
                        if let Some(mut saved) = saved {
                            if matches!(
                                saved.view.status,
                                ProtectedChangeStatus::Pending | ProtectedChangeStatus::Applying
                            ) {
                                saved.view.status = ProtectedChangeStatus::Declined;
                                saved.view.stopped = true;
                            }
                            reviews.entries.insert(saved.view.id.clone(), saved);
                        } else {
                            tracing::warn!("a saved protected-folder review could not be read");
                        }
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => tracing::warn!(%e, "protected-folder reviews could not be opened"),
            }
        }
        reviews
    }
    pub(super) fn save(&mut self, saved: Saved) -> Result<(), String> {
        if let Some(directory) = &self.directory {
            vw_platform::ensure_private_dir(directory).map_err(|e| e.to_string())?;
            let mut bounded = saved.clone();
            let mut remaining = vw_proposals::MAX_RECORD_DIFF_BYTES;
            for file in &mut bounded.view.files {
                if let Some(diff) = &mut file.diff {
                    let mut end = remaining.min(diff.len());
                    while !diff.is_char_boundary(end) {
                        end -= 1;
                    }
                    file.diff_truncated |= end < diff.len();
                    diff.truncate(end);
                    remaining -= end;
                }
            }
            let bytes = serde_json::to_vec(&bounded).map_err(|e| e.to_string())?;
            vw_platform::replace_private(
                &directory.join(format!("{}.json", saved.view.id)),
                &bytes,
            )
            .map_err(|e| e.to_string())?;
        }
        self.entries.insert(saved.view.id.clone(), saved);
        Ok(())
    }
    pub(super) fn overlay(&self, session_id: &str, state: &mut Value) {
        let Some(history) = state.get_mut("history").and_then(Value::as_array_mut) else {
            return;
        };
        history.retain(|entry| entry["type"] != "protected_change");
        let mut reviews: Vec<_> = self
            .entries
            .values()
            .filter(|s| s.view.session_id == session_id)
            .collect();
        reviews.sort_by_key(|s| s.view.created_at_ms);
        // Server history order is authoritative, including entries without timestamps.
        // Weave local reviews before the next dated step without sorting the server's entries.
        for saved in reviews {
            let next = history.iter().position(|entry| {
                entry["createdAt"]
                    .as_u64()
                    .is_some_and(|at| at > saved.view.created_at_ms)
            });
            history.insert(next.unwrap_or(history.len()), history_entry(&saved.view));
        }
    }
}
pub(crate) fn history_entry(view: &ProtectedChangeView) -> Value {
    json!({"id":format!("protected:{}",view.id),"type":"protected_change","createdAt":view.created_at_ms,"turnId":view.turn_id,"proposal":view})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn review(id: &str, session: &str, at: u64) -> Saved {
        Saved {
            view: ProtectedChangeView {
                id: id.into(),
                session_id: session.into(),
                turn_id: Some("turn".into()),
                reason: "Requested change".into(),
                created_at_ms: at,
                decided_at_ms: None,
                status: ProtectedChangeStatus::Pending,
                files: vec![],
                applied: vec![],
                problem: None,
                note: None,
                stopped: false,
            },
            record: None,
        }
    }

    #[test]
    fn no_reviews_leave_the_server_snapshot_byte_for_byte_unchanged() {
        let mut state = json!({"session":{"id":"s"},"history":[
            {"id":"user","type":"message"},
            {"id":"callback","type":"callback","createdAt":1},
            {"id":"assistant","type":"message"}
        ]});
        let original = serde_json::to_vec(&state).unwrap();
        Reviews::default().overlay("s", &mut state);
        assert_eq!(serde_json::to_vec(&state).unwrap(), original);
    }

    #[test]
    fn dated_reviews_keep_server_order_and_repeated_overlays_replace_local_entries() {
        let mut reviews = Reviews::default();
        reviews.save(review("late", "s", 25)).unwrap();
        reviews.save(review("early", "s", 20)).unwrap();
        reviews.save(review("foreign", "other", 15)).unwrap();
        let server = json!([
            {"id":"user","type":"message","createdAt":10},
            {"id":"effect","type":"effect","createdAt":20},
            {"id":"callback","type":"callback"},
            {"id":"assistant","type":"message","createdAt":30}
        ]);
        let mut state = json!({"history":server});
        reviews.overlay("s", &mut state);
        let ids: Vec<_> = state["history"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["id"].as_str().unwrap())
            .collect();
        assert_eq!(
            ids,
            [
                "user",
                "effect",
                "callback",
                "protected:early",
                "protected:late",
                "assistant"
            ]
        );
        let kept: Vec<_> = state["history"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["type"] != "protected_change")
            .cloned()
            .collect();
        assert_eq!(Value::Array(kept), server);
        let first = state.clone();
        reviews.overlay("s", &mut state);
        assert_eq!(state, first);
        reviews.entries.get_mut("early").unwrap().view.status = ProtectedChangeStatus::Applied;
        reviews.overlay("s", &mut state);
        assert_eq!(state["history"].as_array().unwrap().len(), 6);
        assert_eq!(state["history"][3]["proposal"]["status"], "applied");
    }
}
