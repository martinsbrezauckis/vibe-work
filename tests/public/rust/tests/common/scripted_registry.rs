//! Scripted Mistral skill-registry HTTP responses, consumed and validated by the real pinned Vibe.
//! Depends on its RegistrySkillsClient wire (core/skills/registry/models.py), serde and axum. Called by scripted_model.
//! This service has no Vibe methods or session state: real Vibe owns imports, pins, manifests, caching and updates.

use std::collections::BTreeMap;
use std::path::PathBuf;

use axum::Json;
use axum::http::{Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct Registry {
    pub skills: Vec<Skill>,
    /// The test can change {"latest": {"skill-id": 4}} between a review and the install click. With
    /// {"bumpOnPinnedFetch": {"skill-id": 6}} the next fetch of that one skill by a version or alias (the fetch that
    /// installs it, not a newest-version probe) first moves its latest to 6, once.
    pub state_file: Option<PathBuf>,
    /// Skills imported through the real Vibe before the e2e host starts (id -> concrete version).
    pub installed: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Skill {
    pub id: String,
    pub name: String,
    pub description: String,
    pub latest: u64,
}

impl Registry {
    pub fn response(&self, method: &Method, uri: &Uri) -> Option<Response> {
        let path = uri.path();
        if path != "/v1/skills" && !path.starts_with("/v1/skills/") {
            return None;
        }
        if method != Method::GET {
            return Some(StatusCode::METHOD_NOT_ALLOWED.into_response());
        }
        let mut state: Value = self
            .state_file
            .as_ref()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or(Value::Null);
        let pinned_fetch = url::form_urlencoded::parse(uri.query().unwrap_or("").as_bytes())
            .any(|(k, _)| k == "version" || k == "alias");
        if let (Some(file), Some(id)) = (&self.state_file, path.strip_prefix("/v1/skills/"))
            && pinned_fetch
            && let Some(to) = state["bumpOnPinnedFetch"][id].as_u64()
        {
            state["latest"][id] = json!(to);
            if let Some(bumps) = state["bumpOnPinnedFetch"].as_object_mut() {
                bumps.remove(id);
            }
            std::fs::write(file, state.to_string()).ok();
        }
        let latest = |s: &Skill| state["latest"][&s.id].as_u64().unwrap_or(s.latest);
        if path == "/v1/skills" {
            return Some(Json(json!({"data": self.skills.iter().map(|s| payload(s, latest(s), latest(s))).collect::<Vec<_>>(),
                "nextPageToken": ""})).into_response());
        }
        let rest = path.strip_prefix("/v1/skills/")?;
        let id = rest.strip_suffix("/versions").unwrap_or(rest);
        let Some(skill) = self.skills.iter().find(|s| s.id == id) else {
            return Some(StatusCode::NOT_FOUND.into_response());
        };
        let newest = latest(skill);
        if rest.ends_with("/versions") {
            return Some(Json(json!({"items": (1..=newest).rev().map(|v| json!({"version": v,
                "versionAttributes": {"aliases": if v == newest {vec!["stable"]} else {vec![]}}})).collect::<Vec<_>>()})).into_response());
        }
        let version = url::form_urlencoded::parse(uri.query().unwrap_or("").as_bytes())
            .find_map(|(k, v)| (k == "version").then(|| v.parse::<u64>().ok()).flatten())
            .unwrap_or(newest);
        if version == 0 || version > newest {
            return Some(StatusCode::NOT_FOUND.into_response());
        }
        Some(Json(payload(skill, version, newest)).into_response())
    }
}

fn payload(skill: &Skill, version: u64, newest: u64) -> Value {
    json!({"skillId": skill.id, "version": version,
        "skill": {"skillName": skill.name, "skillDescription": skill.description,
            "skillBody": format!("# {}\nScripted registry version {version}.\n", skill.name)},
        "attributes": {"title": skill.name, "description": skill.description},
        "metadata": {"name": skill.name, "latestVersion": newest, "sharingScope": "workspace"},
        "versionAttributes": {"notes": format!("Version {version}"), "aliases": []}})
}
