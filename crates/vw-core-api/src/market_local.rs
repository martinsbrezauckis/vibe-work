//! Joins Vibe discovery with a chat's pinned catalog for the local plugin section.
//! Called by `market`; depends on Vibe's public projections and marketplace receipts.

use std::path::Path;

use serde_json::Value;
use vw_market::Store;

use crate::market_view::{ComponentView, LocalPluginView, PluginServerView};

fn source_path(row: &Value) -> std::path::PathBuf {
    let path = Path::new(text(row, "path"));
    if row["off"].as_bool() == Some(true)
        && let (Some(parent), Some(name)) = (path.parent().and_then(Path::parent), path.file_name())
    {
        return parent.join("plugins").join(name);
    }
    path.to_path_buf()
}

fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// An app receipt identifies a catalog install even while its folder is off.
pub(crate) fn receipt(folder: &Path) -> Option<vw_market::Receipt> {
    vw_market::Receipt::read_from(folder)
}

/// Only direct, non-linked plugin folders in these two owned roots can be changed.
pub(crate) fn managed(path: &Path, home: &Path, project: Option<&Path>, off: bool) -> bool {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    for variable in ["HOME", "USERPROFILE"] {
        if let Some(user_home) = std::env::var_os(variable) {
            let cli = std::path::PathBuf::from(user_home).join(".vibe");
            let cli = cli.canonicalize().unwrap_or(cli);
            if canonical.starts_with(cli) {
                return false;
            }
        }
    }
    let roots = [
        Some(home.join(if off { "plugins-off" } else { "plugins" })),
        project.map(|p| {
            p.join(".vibe")
                .join(if off { "plugins-off" } else { "plugins" })
        }),
    ];
    roots.into_iter().flatten().any(|root| {
        // Refuse links at every segment below the owned home/project, including junctions.
        let base = if root.starts_with(home) {
            home
        } else {
            project.unwrap_or(home)
        };
        let Ok(relative) = root.strip_prefix(base) else {
            return false;
        };
        let mut part = base.to_path_buf();
        for segment in relative.components() {
            part.push(segment);
            if std::fs::symlink_metadata(&part).is_ok_and(|m| m.file_type().is_symlink()) {
                return false;
            }
        }
        path.parent()
            .is_some_and(|parent| parent.canonicalize().ok() == root.canonicalize().ok())
            && std::fs::symlink_metadata(path)
                .is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
            && root.canonicalize().is_ok_and(|canonical| {
                path.canonicalize()
                    .is_ok_and(|p| p.parent() == Some(canonical.as_path()))
            })
    })
}

/// All discovered non-receipted plugins, plus pins whose source is no longer installed.
pub(crate) fn project(
    discovery: Option<&Value>,
    pinned: &Value,
    skills: &Value,
    store: &Store,
    home: &Path,
    cwd: Option<&Path>,
    catalog: Option<&vw_market::Catalog>,
) -> Vec<LocalPluginView> {
    let mut rows: Vec<Value> = discovery
        .and_then(|d| d["plugins"].as_array())
        .cloned()
        .unwrap_or_default();
    let pins = pinned.pointer("/plugins/plugins").and_then(Value::as_array);
    for pin in pins.into_iter().flatten() {
        let path = text(pin, "installedRoot");
        let path = if path.is_empty() {
            text(pin, "pinnedRoot")
        } else {
            path
        };
        if !rows.iter().any(|row| source_path(row) == Path::new(path)) {
            let mut row = pin.clone();
            row["path"] = Value::String(path.to_owned());
            rows.push(row);
        }
    }
    let receipts = store.installed();
    let mut result: Vec<_> = rows
        .iter()
        .filter_map(|row| {
            let name = text(row, "name");
            let path = text(row, "path");
            // Rejected folders must keep their unreadable card even with a marketplace receipt.
            if row["readable"] != false
                && ((receipt(Path::new(path))
                    .is_some_and(|r| catalog.is_some_and(|c| c.entry(&r.entry_id).is_some()))
                    && Path::new(path).parent().is_some_and(|root| {
                        root == store.plugins_dir()
                            || root == store.plugins_dir().with_file_name("plugins-off")
                    }))
                    || receipts.keys().any(|id| {
                        catalog.is_some_and(|c| c.entry(id).is_some())
                            && store
                                .plugin_dir(id)
                                .is_ok_and(|folder| folder == Path::new(path))
                    }))
            {
                return None;
            }
            let off = row["off"].as_bool().unwrap_or(false);
            let pin = pins.into_iter().flatten().find(|p| {
                text(p, "name") == name
                    && (Path::new(text(p, "installedRoot")) == source_path(row)
                        || Path::new(text(p, "pinnedRoot")) == source_path(row)
                        || text(p, "installedRoot").is_empty() && text(p, "pinnedRoot").is_empty())
            });
            let mut in_session = (!pinned.is_null())
                .then(|| crate::market_project::session_plugin(name, pinned, skills));
            if pin.is_none()
                && let Some(state) = &mut in_session
            {
                state.loaded = false;
                state.components.clear();
                state.skills.clear();
                state.issues.clear();
            }
            let components: Vec<ComponentView> = row["components"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|c| ComponentView {
                    kind: text(c, "kind").to_owned(),
                    name: text(c, "name").to_owned(),
                    status: c["status"].as_str().map(str::to_owned),
                })
                .collect();
            let servers = servers(row, pin, &components);
            Some(LocalPluginView {
                name: name.to_owned(),
                description: text(row, "description").to_owned(),
                version: row["version"].as_str().map(str::to_owned),
                author: row["author"].as_str().map(str::to_owned),
                path: path.to_owned(),
                scope: text(row, "scope").to_owned(),
                managed: managed(Path::new(path), home, cwd, off),
                off,
                available: row["available"].as_bool().unwrap_or(false),
                components,
                servers,
                in_session,
                issues: row["issues"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect(),
            })
        })
        .collect();
    result.sort_by(|a, b| a.name.cmp(&b.name).then(a.path.cmp(&b.path)));
    result
}

fn servers(
    row: &Value,
    pin: Option<&Value>,
    components: &[ComponentView],
) -> Vec<PluginServerView> {
    components
        .iter()
        .filter(|c| c.kind == "mcp_server")
        .map(|c| {
            let group = row["servers"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|server| text(server, "name") == c.name)
                .and_then(|server| server["group"].as_str());
            PluginServerView {
                name: c.name.clone(),
                tools: group.and_then(|group| {
                    pin.and_then(|p| p["components"].as_array()).map(|items| {
                        items
                            .iter()
                            .filter(|tool| {
                                text(tool, "kind") == "tool"
                                    && text(tool, "name")
                                        .strip_prefix(group)
                                        .is_some_and(|suffix| suffix.starts_with('.'))
                            })
                            .count()
                    })
                }),
            }
        })
        .collect()
}
