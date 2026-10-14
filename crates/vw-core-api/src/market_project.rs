//! Turning the catalog, the receipts and Vibe's replies into the marketplace view, and the consent
//! dialog's sentence.
//!
//! Depends on: vw-market, `market_view`, `serde_json`. Called by: `market`.
//! Invariants:
//! - Vibe's replies are read defensively (missing fields = unknown, never a panic); only names, statuses and Vibe's
//!   own issue sentences are copied; a dropped plugin's file is shown by its last two path parts only.
//! - Every quoted catalog value in the dialog goes through `vw_gate::dialog_value`, preventing spoofed lines.

use serde_json::Value;
use vw_market::{Catalog, Entry, Kind, Store};

use crate::market_view::{
    CategoryView, ComponentView, DroppedView, InstallProgress, InstalledView, MarketEntryView,
    MarketView, SessionPluginView, SkillStateView,
};

/// What [`view`] needs.
pub(crate) struct Inputs<'a> {
    pub(crate) catalog: &'a Catalog,
    pub(crate) store: &'a Store,
    /// `plugin_catalog/read`'s result (or `null`).
    pub(crate) plugins: &'a Value,
    /// `skills/installed`'s result (or `null`).
    pub(crate) skills: &'a Value,
    pub(crate) session_id: Option<String>,
    pub(crate) installing: Option<InstallProgress>,
}

fn categories() -> Vec<CategoryView> {
    vw_market::catalog::CATEGORIES
        .iter()
        .map(|(id, label)| CategoryView {
            id: (*id).to_owned(),
            label: (*label).to_owned(),
        })
        .collect()
}

/// The view when the catalog could not be read.
pub(crate) fn unavailable(error: &str) -> MarketView {
    MarketView {
        catalog_version: None,
        error: Some(error.to_owned()),
        categories: categories(),
        entries: Vec::new(),
        platform: vw_market::current_target().map(str::to_owned),
        installing: None,
        session_id: None,
        dropped: Vec::new(),
    }
}

/// The marketplace view.
pub(crate) fn view(inputs: Inputs<'_>) -> MarketView {
    let installed = inputs.store.installed();
    let session_read = !inputs.plugins.is_null();
    let entries = inputs
        .catalog
        .entries
        .iter()
        .map(|entry| {
            let receipt = installed.get(&entry.id);
            let foreign = entry.kind != Kind::Server
                && receipt.is_none()
                && inputs.store.plugin_dir(&entry.id).is_ok_and(|d| d.exists());
            MarketEntryView {
                installed: receipt.map(|r| InstalledView {
                    version: r.version.clone(),
                    catalog_version: r.catalog_version.clone(),
                    installed_at_ms: r.installed_at_ms,
                    update_available: r.version != entry_version(entry),
                    changed: (entry.kind == Kind::Pack).then(|| inputs.store.changed(r)),
                }),
                foreign,
                installable: crate::market::installable(entry),
                in_session: (session_read && entry.kind != Kind::Server)
                    .then(|| session_plugin(&entry.id, inputs.plugins, inputs.skills)),
                entry: entry.clone(),
            }
        })
        .collect();
    MarketView {
        catalog_version: Some(inputs.catalog.version.clone()),
        error: None,
        categories: categories(),
        entries,
        platform: vw_market::current_target().map(str::to_owned),
        installing: inputs.installing,
        session_id: inputs.session_id,
        dropped: dropped(inputs.plugins),
    }
}

fn entry_version(entry: &Entry) -> String {
    match (&entry.pack, &entry.plugin) {
        (Some(pack), _) => pack.version.clone(),
        (_, Some(plugin)) => plugin.version.clone(),
        _ => String::new(),
    }
}

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// The plugin `name` as the session runs it (`plugin_catalog/read` -> `plugins.plugins[]`, `models.py`
/// `PluginCatalogEntry`, components named `<namespace>:<skill>`; `skills/installed` -> `skills[]`, `SkillSummary`,
/// same names). After `plugin/reload`, a new plugin and its skill components are in
/// the catalog, but `skills/installed` lists its skills only in sessions started afterwards.
fn session_plugin(name: &str, plugins: &Value, skills: &Value) -> SessionPluginView {
    let list = plugins
        .pointer("/plugins/plugins")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let found = list
        .iter()
        .find(|p| p.get("name").and_then(Value::as_str) == Some(name));
    let components: Vec<ComponentView> = found
        .and_then(|p| p.get("components"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|c| ComponentView {
                    kind: text(c, "kind"),
                    name: text(c, "name"),
                    status: c.get("status").and_then(Value::as_str).map(str::to_owned),
                })
                .collect()
        })
        .unwrap_or_default();
    let offered: Vec<&Value> = skills
        .get("skills")
        .and_then(Value::as_array)
        .map(|items| items.iter().collect())
        .unwrap_or_default();
    let skills = components
        .iter()
        .filter(|c| c.kind == "skill")
        .map(|c| {
            let row = offered
                .iter()
                .find(|s| s.get("name").and_then(Value::as_str) == Some(c.name.as_str()));
            SkillStateView {
                name: c.name.clone(),
                description: row.map(|r| text(r, "description")).unwrap_or_default(),
                enabled: row.map(|r| r.get("enabled").and_then(Value::as_bool).unwrap_or(true)),
                locked: row
                    .is_some_and(|r| r.get("locked").and_then(Value::as_bool).unwrap_or(false)),
            }
        })
        .collect();
    let pinned = found.map(|p| text(p, "pinnedRoot")).unwrap_or_default();
    let issues = dropped_items(plugins)
        .filter(|d| !pinned.is_empty() && text(d, "file").starts_with(&pinned))
        .map(|d| text(d, "message"))
        .collect();
    SessionPluginView {
        loaded: found.is_some(),
        components,
        skills,
        issues,
    }
}

fn dropped_items(plugins: &Value) -> impl Iterator<Item = &Value> {
    plugins
        .pointer("/plugins/dropped")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
}

fn dropped(plugins: &Value) -> Vec<DroppedView> {
    let roots: Vec<(String, String)> = plugins
        .pointer("/plugins/plugins")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|p| (text(p, "pinnedRoot"), text(p, "name")))
                .filter(|(root, _)| !root.is_empty())
                .collect()
        })
        .unwrap_or_default();
    dropped_items(plugins)
        .map(|d| {
            let file = text(d, "file");
            DroppedView {
                plugin: roots
                    .iter()
                    .find(|(root, _)| file.starts_with(root.as_str()))
                    .map(|(_, name)| name.clone()),
                file: last_parts(&file),
                message: text(d, "message"),
            }
        })
        .collect()
}

/// The last two parts of a path (`plugins/notes-pack/plugin.json` -> `notes-pack/plugin.json`).
fn last_parts(path: &str) -> String {
    let parts: Vec<&str> = path.split(['/', '\\']).filter(|p| !p.is_empty()).collect();
    parts[parts.len().saturating_sub(2)..].join("/")
}

/// The native dialog's sentence for installing `entry`.
pub(crate) fn consent_summary(entry: &Entry) -> String {
    let quote = |text: &str| vw_gate::dialog_quote(text);
    match (&entry.pack, &entry.plugin) {
        (Some(pack), _) => {
            let skills = pack
                .files
                .iter()
                .filter(|f| f.to.ends_with("/SKILL.md"))
                .count();
            let from = pack.source.as_ref().map_or_else(String::new, |s| {
                format!("; files from {} at {}", quote(&s.repo), quote(&s.commit))
            });
            let servers = if pack.mcp_servers.is_empty() {
                String::new()
            } else {
                format!(
                    "; connects {} server(s) that need no sign-in",
                    pack.mcp_servers.len()
                )
            };
            format!(
                "Install {} by {} ({} licence) into Vibe: {skills} skill(s){from}{servers}",
                quote(&entry.name),
                quote(&entry.publisher),
                quote(&entry.licence)
            )
        }
        (_, Some(plugin)) => format!(
            "Install {} by {} ({} licence): {} About {} MB download, {} MB on disk.",
            quote(&entry.name),
            quote(&entry.publisher),
            quote(&entry.licence),
            quote(&plugin.consent),
            plugin.download_mb,
            plugin.disk_mb
        ),
        _ => format!("Install {}", quote(&entry.name)),
    }
}
