//! `--edit-files` (the fixture): an approved turn whose user message contains `edit:<relative path>` tokens
//! appends one line to each named file in the session folder (creating it when missing) and records a
//! `state.output = {filePath, content, fileExisted, previousContent}`), so the Changes tab has real disk
//! changes and real effect data to attribute.

use serde_json::{Value, json};

use crate::script::Script;

impl Script {
    /// Reproduces legacy Vibe rewind: restore the suffix's writes, retain earlier history, clear turns.
    pub fn rewind_edits(&self, sid: &str, mut state: Value, params: &Value) -> Value {
        let Some(history) = state["history"].as_array_mut() else {
            return json!({"__error": "not_found"});
        };
        let Some(cut) = history
            .iter()
            .position(|entry| entry["id"] == params["entryId"])
        else {
            return json!({"__error": "not_found"});
        };
        let mut restored = Vec::new();
        if params["restoreFiles"] == true {
            for entry in history[cut..].iter().rev() {
                if entry.pointer("/detail/kind").and_then(Value::as_str) != Some("file_write") {
                    continue;
                }
                let output = &entry["state"]["output"];
                let path = output["filePath"].as_str().unwrap();
                if let Some(previous) = output["previousContent"].as_str() {
                    std::fs::write(path, previous).unwrap();
                } else {
                    std::fs::remove_file(path).unwrap();
                }
                restored.push(path.to_owned());
            }
        }
        history.truncate(cut);
        state["turns"] = json!([]);
        self.world
            .lock()
            .unwrap()
            .sessions
            .insert(sid.to_owned(), state.clone());
        json!({"state": state, "message": "rewound", "restoreErrors": [],
            "restoredPaths": restored, "sessionLog": {}})
    }

    /// Applies the `edit:` tokens of `params.message` for an approved turn (no-op without the flag).
    pub fn apply_edits(&self, sid: &str, turn_id: &str, params: &Value, decision: &str) {
        if !self.edit_files || !decision.starts_with("approve") {
            return;
        }
        let Some(cwd) = self
            .state(sid)
            .and_then(|s| s["session"]["cwd"].as_str().map(str::to_owned))
        else {
            return;
        };
        let text: String = params["message"]
            .as_array()
            .map(|blocks| {
                blocks
                    .iter()
                    .filter_map(|b| b["text"].as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default();
        for rel in text
            .split_whitespace()
            .filter_map(|w| w.strip_prefix("edit:"))
        {
            if rel.is_empty() || rel.contains("..") || rel.starts_with('/') {
                continue;
            }
            let path = std::path::Path::new(&cwd).join(rel);
            let previous = std::fs::read_to_string(&path).ok();
            let content = format!(
                "{}edited in {turn_id}\n",
                previous.clone().unwrap_or_default()
            );
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if std::fs::write(&path, &content).is_err() {
                continue;
            }
            let id = self.next("effect");
            let file = path.to_string_lossy().into_owned();
            self.add_entry(
                sid,
                json!({"id": id, "type": "effect", "sessionId": sid, "turnId": turn_id,
                    "generationStatus": "completed", "createdAt": 1, "updatedAt": 1,
                    "detail": {"kind": "file_write", "toolName": "write_file",
                        "display": {"verb": "Write", "message": rel},
                        "input": {"filePath": file, "content": content, "overwrite": true}},
                    "state": {"status": "completed",
                        "output": {"filePath": file, "content": content,
                            "fileExisted": previous.is_some(), "previousContent": previous},
                        "display": {"success": true, "verb": "Wrote", "message": rel}}}),
            );
        }
    }
}
