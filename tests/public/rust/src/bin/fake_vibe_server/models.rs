//! Optional model profile for the scripted fake. It reads only the test home's explicit config path;
//! shell_host enables it for the composer regression, while every other fake run keeps cli-fast.

use std::collections::BTreeMap;
use std::sync::Mutex;

use serde_json::{Value, json};

/// The per-session model choice and a synthetic configuration view.
pub struct Models {
    config: Mutex<Value>,
    chosen: Mutex<BTreeMap<String, (String, String)>>,
    log: Option<Mutex<std::fs::File>>,
}

impl Models {
    /// Builds the opt-in profile from the test home's config.toml.
    pub fn from_path(path: &str, log_path: Option<&str>) -> Self {
        let source = std::fs::read_to_string(path).expect("model profile config");
        let parsed: toml::Value = source.parse().expect("model profile TOML");
        let local = parsed
            .get("models")
            .and_then(toml::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|m| {
                Some(json!({
                    "alias": m.get("alias")?.as_str()?,
                    "name": m.get("name")?.as_str()?,
                    "displayName": m.get("display_name").and_then(toml::Value::as_str).or_else(|| m.get("alias").and_then(toml::Value::as_str)),
                    "thinking": m.get("thinking").and_then(toml::Value::as_str).unwrap_or("off"),
                    "supportsImages": m.get("supports_images").and_then(toml::Value::as_bool).unwrap_or(false),
                    "maxContextLength": m.get("max_context_length").and_then(toml::Value::as_integer),
                    "thinkingLevels": ["off", "low", "medium", "high", "max"]
                }))
            });
        let models: Vec<Value> = local
            .chain([
                json!({"alias": "mistral-medium-3.5", "name": "mistral-vibe-cli-latest", "displayName": "Medium 3.5", "thinking": "high", "thinkingLevels": ["off", "high"]}),
                json!({"alias": "glm-5-3", "name": "zai-glm-5-3", "displayName": "GLM-5.3", "thinking": "high", "thinkingLevels": ["off", "low", "medium", "high", "max"]}),
            ])
            .collect();
        let active = parsed
            .get("active_model")
            .and_then(toml::Value::as_str)
            .expect("active_model in test config");
        let config = json!({"models": models, "activeModel": {"alias": active, "thinking": "off"},
            "activeModelPinned": true, "defaultModelAlias": "glm-5-3"});
        let log = log_path.map(|path| {
            Mutex::new(
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                    .expect("model log"),
            )
        });
        Self {
            config: Mutex::new(config),
            chosen: Mutex::new(BTreeMap::new()),
            log,
        }
    }

    /// Returns the config with this session's current pick, if any.
    pub fn config(&self, sid: &str) -> Value {
        let mut config = self.config.lock().unwrap().clone();
        if let Some((alias, effort)) = self.chosen.lock().unwrap().get(sid) {
            config["activeModel"] = json!({"alias": alias, "thinking": effort});
        }
        config
    }

    /// Records an explicit model write for the session.
    pub fn write(&self, sid: &str, params: &Value) -> bool {
        let current = self.config(sid);
        let alias = params["modelAlias"].as_str().unwrap_or_else(|| {
            current["activeModel"]["alias"]
                .as_str()
                .unwrap_or("large-4")
        });
        let mut config = self.config.lock().unwrap();
        let model = config["models"]
            .as_array()
            .and_then(|models| models.iter().find(|model| model["alias"] == alias));
        let Some(model) = model else { return false };
        let default_effort = model["thinking"].as_str().unwrap_or("off");
        let effort = params["reasoningEffort"].as_str().unwrap_or(default_effort);
        let alias = alias.to_owned();
        let effort = effort.to_owned();
        if params["reasoningEffort"].is_string()
            && let Some(model) = config["models"]
                .as_array_mut()
                .and_then(|models| models.iter_mut().find(|model| model["alias"] == alias))
        {
            model["thinking"] = json!(effort);
        }
        config["activeModel"] = json!({"alias": alias, "thinking": effort});
        config["activeModelPinned"] = json!(true);
        drop(config);
        self.chosen
            .lock()
            .unwrap()
            .insert(sid.to_owned(), (alias, effort));
        true
    }

    /// Records what the fake will use at the start of a turn, before its worker runs.
    pub fn log_turn(&self, sid: &str) {
        if let Some(log) = &self.log {
            let config = self.config(sid);
            let mut file = log.lock().unwrap();
            use std::io::Write;
            let _ = writeln!(
                file,
                "{}",
                json!({"sessionId": sid, "activeModel": config["activeModel"]})
            );
        }
    }
}
