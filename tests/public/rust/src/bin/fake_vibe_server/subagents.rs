//! Synthetic subagent-setting responses for core command tests. `--subagents-rule` saves a
//! setting through a new helper chat; `--subagents-reject` rejects without saving.
//!
//! Depends on: `script` (world, runtime). Called by: `script` (`config/write`, `config/fields/read`).

use serde_json::{Value, json};

use crate::script::Script;

impl Script {
    /// `config/write`: saves `/enable_subagents` through a new helper chat.
    pub fn config_write_subagents(&self, sid: &str, params: &Value) -> Value {
        if self.subagents_reject {
            return json!({"runtime": self.runtime(sid), "rejected": true});
        }
        #[cfg(feature = "fixture-extensions")]
        if let Some(answer) = self.fixture_subagents_write(sid, params) {
            return answer;
        }
        let value = params["ops"].as_array().and_then(|ops| {
            ops.iter()
                .rev()
                .find(|op| op["op"] == "set" && op["path"] == "/enable_subagents")
                .and_then(|op| op["value"].as_bool())
        });
        if let Some(value) = value {
            let mut world = self.world.lock().unwrap();
            if !world.unstored.contains(sid) {
                return json!({"__error": "internal_error",
                              "__message": "Synthetic setting changes require a new chat."});
            }
            world.subagents = Some(value);
        }
        json!({"runtime": self.runtime(sid)})
    }

    /// `config/fields/read`: the saved value of `enable_subagents` in the user's own layer.
    pub fn config_fields_subagents(&self) -> Value {
        let value = self.world.lock().unwrap().subagents.unwrap_or(true);
        json!({"fields": [{"name": "enable_subagents", "value": value,
                           "layerValues": [{"layer": "user-toml", "value": value}]}],
               "targets": []})
    }
}
