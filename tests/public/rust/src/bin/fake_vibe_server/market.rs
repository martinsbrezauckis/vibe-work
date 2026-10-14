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

#[derive(Default)]
struct World {
    pinned: BTreeMap<String, Vec<(String, Vec<String>)>>,
}

fn world() -> &'static Mutex<World> {
    static WORLD: OnceLock<Mutex<World>> = OnceLock::new();
    WORLD.get_or_init(Mutex::default)
}

/// The user plugins on disk now: (name, skill names).
fn scan() -> Vec<(String, Vec<String>)> {
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
        out.push((
            name,
            skills
                .into_iter()
                .map(|s| format!("{namespace}:{s}"))
                .collect(),
        ));
    }
    out.sort();
    out
}

/// Answers one of [`HANDLES`].
pub fn call(method: &str, params: &Value) -> Value {
    let sid = params["sessionId"].as_str().unwrap_or_default().to_owned();
    let mut w = world().lock().unwrap();
    if method == "plugin/reload" {
        w.pinned.insert(sid, scan());
        return json!({});
    }
    let pinned = w.pinned.entry(sid).or_insert_with(scan).clone();
    match method {
        "plugin_catalog/read" => json!({"plugins": {
            "plugins": pinned.iter().map(|(name, skills)| json!({
                "name": name, "version": "1", "sourceFormat": "native", "manifestDigest": "fake",
                "description": "", "scope": "global",
                "components": skills.iter().map(|s| json!({"kind": "skill", "name": s, "status": null})).collect::<Vec<_>>(),
                "drifted": 0})).collect::<Vec<_>>(),
            "dropped": [{"file": "/home/x/.vibe-home/plugins/broken/plugin.json", "message": "Failed to load plugin manifest"}]}}),
        "skills/installed" => {
            json!({"skills": pinned.iter().flat_map(|(_, skills)| skills.iter()).map(|s| json!({
            "name": s, "description": format!("{s} skill"), "prompt": "", "source": "plugin", "scope": "global",
            "enabled": true, "locked": true})).collect::<Vec<_>>()})
        }
        _ => json!({"__error": "method_not_found"}),
    }
}
