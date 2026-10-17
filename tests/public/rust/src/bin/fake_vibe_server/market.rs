//! The fake's plugins and skills (the fixture), shaped like Vibe 2.26.x (`plugin_catalog.py`, `_skills_service.py`,
//! `models.py` `PluginCatalogEntry` / `SkillSummary`):
//! - a session pins the user plugins in `$VIBE_HOME/plugins/*/plugin.json` the first time it is asked and keeps that
//!   snapshot until `plugin/reload` (like Vibe's per-session pin), so a test sees whether the core reloaded;
//! - `plugin_catalog/read` lists the pinned plugins with one `skill` component per `skills/*/SKILL.md`, named
//!   `<plugin name with _ for ->:<skill>`;
//! - `skills/installed` lists those skills as Vibe's unified backend does: `source: plugin`, `enabled: true`,
//!   `locked: true` (`_unified_harness_backend_adapter.py:1303`).

use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};

use serde_json::{Value, json};

/// The methods this module answers.
pub const HANDLES: &[&str] = &["plugin_catalog/read", "plugin/reload", "skills/installed"];

#[derive(Clone)]
struct Plugin {
    name: String,
    skills: Vec<String>,
    servers: Vec<String>,
}
#[derive(Default)]
struct World {
    pinned: BTreeMap<String, Vec<Plugin>>,
}

fn world() -> &'static Mutex<World> {
    static WORLD: OnceLock<Mutex<World>> = OnceLock::new();
    WORLD.get_or_init(Mutex::default)
}

/// The user plugins on disk now: (name, skill names).
fn scan() -> Vec<Plugin> {
    let Some(home) = std::env::var_os("VIBE_HOME") else {
        return Vec::new();
    };
    let root = std::path::Path::new(&home).join("plugins");
    let mut out = Vec::new();
    for item in std::fs::read_dir(root).into_iter().flatten().flatten() {
        let Ok(text) = std::fs::read_to_string(item.path().join("plugin.json")) else {
            continue;
        };
        let Ok(manifest) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        let name = manifest["name"].as_str().unwrap_or_default().to_owned();
        let mut skills: Vec<String> = std::fs::read_dir(item.path().join("skills"))
            .into_iter()
            .flatten()
            .flatten()
            .filter(|s| s.path().join("SKILL.md").is_file())
            .map(|s| s.file_name().to_string_lossy().into_owned())
            .collect();
        skills.sort();
        let namespace = name.replace('-', "_");
        let servers = std::fs::read_to_string(item.path().join("mcp.json"))
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .and_then(|config| {
                config["mcpServers"]
                    .as_object()
                    .map(|servers| servers.keys().cloned().collect())
            })
            .unwrap_or_default();
        out.push(Plugin {
            name,
            skills: skills
                .into_iter()
                .map(|s| format!("{namespace}:{s}"))
                .collect(),
            servers,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Answers one of [`HANDLES`].
pub fn call(method: &str, params: &Value) -> Value {
    let sid = params["sessionId"].as_str().unwrap_or_default().to_owned();
    let mut w = world().lock().unwrap();
    if method == "plugin/reload" {
        if std::env::var_os("VIBE_HOME").is_some_and(|home| {
            std::path::Path::new(&home)
                .join("refuse-plugin-reload")
                .exists()
        }) {
            return json!({"__error":"invalid_request", "__message":"cannot re-pin the plugins of a non-quiescent session"});
        }
        w.pinned.insert(sid, scan());
        return json!({});
    }
    let pinned = w.pinned.entry(sid).or_insert_with(scan).clone();
    match method {
        "plugin_catalog/read" => json!({"plugins": {
            "plugins": pinned.iter().map(|plugin| { let name = &plugin.name; json!({
                "name": name, "version": "1", "sourceFormat": "native", "manifestDigest": "fake",
                "description": "", "scope": "global",
                "pinnedRoot": std::env::var_os("VIBE_HOME").map(|home| std::path::Path::new(&home).join("plugins").join(name)),
                "installedRoot": std::env::var_os("VIBE_HOME").map(|home| std::path::Path::new(&home).join("plugins").join(name)),
                "components": components(plugin),
                "drifted": 0}) }).collect::<Vec<_>>(),
            "dropped": [{"file": "/home/x/.vibe-home/plugins/broken/plugin.json", "message": "Failed to load plugin manifest"}]}}),
        "skills/installed" => {
            json!({"skills": pinned.iter().flat_map(|plugin| plugin.skills.iter()).map(|s| json!({
            "name": s, "description": format!("{s} skill"), "prompt": "", "source": "plugin", "scope": "global",
            "enabled": true, "locked": true})).collect::<Vec<_>>()})
        }
        _ => json!({"__error": "method_not_found"}),
    }
}

fn components(plugin: &Plugin) -> Vec<Value> {
    let mut components: Vec<_> = plugin
        .skills
        .iter()
        .map(|s| json!({"kind":"skill", "name":s, "status":null}))
        .collect();
    for server in &plugin.servers {
        components.push(json!({"kind":"mcp_server", "name":server, "status":null}));
        for i in 0..2 {
            components.push(json!({"kind":"tool", "name":format!("plugin_{}_{}.tool{i}", plugin.name.replace('-', "_"), server), "status":null}));
        }
    }
    components
}

/// Records the actual folders at the instant the fake handles a loading request.
pub fn record_load(method: &str) {
    use std::io::Write as _;
    if !matches!(
        method,
        "server/start" | "session/start" | "session/resume" | "plugin/reload" | "config/read"
    ) {
        return;
    }
    let Some(home) = std::env::var_os("VIBE_HOME") else {
        return;
    };
    let home = std::path::Path::new(&home);
    let folders: Vec<_> = std::fs::read_dir(home.join("plugins"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    let mut log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(home.join("plugin-loads.jsonl"))
        .unwrap();
    writeln!(log, "{}", json!({"method": method, "folders": folders})).unwrap();
}
