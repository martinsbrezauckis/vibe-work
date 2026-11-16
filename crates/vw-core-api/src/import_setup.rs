//! `importSetup`: bring the person's setup from other coding agents into Vibe Work's own home.
//!
//! - `scan` reads the other agents' setups (`vw-import`; read only, sign-in and secret files never opened).
//! - `apply` scans again, refuses a picked row whose source changed since the person saw it, shows ONE native
//!   confirmation that lists every file it will write and every key that goes to the vault, keeps a backup of every
//!   file it changes, then writes: instructions appended under a heading to the personal `AGENTS.md`, skill folders
//!   copied into the skills folder, MCP servers added to Vibe's config. A server's key goes into the vault as a key
//!   server (permission Ask; the server is not allowed until the person allows it, as for a server added in Tools)
//!   and Vibe's config holds only the forwarder's address.
//! - `last` answers the newest bring of this run that can still be undone, so the window shows its Undo again.
//! - `undo` puts every changed file back byte for byte, removes the copied skills and the key servers it added,
//!   within 24 hours, for a bring of this run only: what a bring changed is kept in the core's memory ([`Brings`]),
//!   never read back from `VIBE_HOME/import-backup/<bring>/`, which an agent can edit (that folder holds only the
//!   backup copies), so a written or edited file there cannot make Undo remove another vault secret.
//!
//! The Debug API may scan; `apply` and `undo` are the person's own acts (`vault_scope`: user-only), because a bring
//! stores keys. Nothing in the other agents' folders is changed. Key values are never part of a result, a dialog or
//! a log line.
//!
//! Depends on: vw-import, `mcp_keys` (key servers), `mcp_keys_config` (config entries), the native confirmer.
//! Called by: `dispatch`.

use std::path::{Path, PathBuf};

use serde_json::json;
use vw_import::mcp::Server;
use vw_import::{Found, Homes, Kind, Payload, Status, Taken};

use vw_platform::sync::LockRecover;

use crate::command::Origin;
use crate::core::Core;
use crate::error::{CoreError, ErrorCode};
use crate::import_view::{ImportPick, ImportRequest, ImportResult, ImportSummary};

/// The native confirmation's method name.
pub(crate) const METHOD: &str = "vw/import/apply";
/// How long a bring can be undone.
const UNDO_MS: u64 = 24 * 60 * 60 * 1000;

fn invalid(message: impl Into<String>) -> CoreError {
    CoreError::new(ErrorCode::InvalidParams, message)
}

fn failed(what: &str, error: impl std::fmt::Display) -> CoreError {
    CoreError::new(ErrorCode::Internal, format!("{what}: {error}"))
}

fn now_ms() -> u64 {
    vw_changes::now_ms()
}

/// The person's home and the config folder the other agents use.
fn homes(core: &Core, project: Option<&str>) -> Option<Homes> {
    let fixed = core
        .shared
        .import_home
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    let env = |name: &str| {
        std::env::var_os(name)
            .filter(|_| fixed.is_none())
            .map(PathBuf::from)
    };
    let home = fixed
        .clone()
        .or_else(|| env(if cfg!(windows) { "USERPROFILE" } else { "HOME" }))?;
    Some(Homes {
        config_dir: env("XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config")),
        codex_home: env("CODEX_HOME"),
        home,
        project: project.map(PathBuf::from),
    })
}

/// Names Vibe Work's own setup already uses.
fn taken(home: &Path) -> Taken {
    let skills = std::fs::read_dir(home.join("skills"))
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    let servers = crate::connectors_config::configured_servers(home)
        .into_iter()
        .map(|s| s.name)
        .collect();
    Taken { skills, servers }
}

/// A relative path inside a copied skill folder: plain parts only.
fn safe_relative(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
}

/// The backup folder of one bring.
fn backup_dir(home: &Path, bring: &str) -> Result<PathBuf, CoreError> {
    if bring.is_empty() || !bring.bytes().all(|b| b.is_ascii_digit()) {
        return Err(invalid("not a bring id"));
    }
    Ok(home.join("import-backup").join(bring))
}

impl Core {
    pub(crate) async fn import_setup(
        &self,
        origin: Origin,
        request: ImportRequest,
    ) -> Result<ImportResult, CoreError> {
        let home = self.shared.instructions.own_home.clone();
        match request {
            ImportRequest::Detect {} => {
                let homes =
                    homes(self, None).ok_or_else(|| invalid("The home folder is not known."))?;
                let agents = crate::worktrees::blocking(move || {
                    vw_import::detect(&vw_import::fs::Disk, &homes)
                })
                .await?
                .into_iter()
                .map(|agent| crate::import_view::ImportDetected {
                    agent,
                    label: agent.label().to_owned(),
                })
                .collect();
                Ok(ImportResult::Detected { agents })
            }
            ImportRequest::Scan { project } => {
                let homes = homes(self, project.as_deref())
                    .ok_or_else(|| invalid("The home folder is not known."))?;
                let rows = crate::worktrees::blocking(move || {
                    vw_import::scan(&vw_import::fs::Disk, &homes, &taken(&home))
                })
                .await?;
                Ok(ImportResult::Plan { rows })
            }
            ImportRequest::Apply { picks, project } => {
                self.import_apply(origin, &home, &picks, project.as_deref())
                    .await
            }
            ImportRequest::Last {} => Ok(ImportResult::Last {
                summary: last_bring(self),
            }),
            ImportRequest::Undo { bring } => {
                import_undo(self, &home, &bring).await?;
                Ok(ImportResult::Undone { bring })
            }
        }
    }

    async fn import_apply(
        &self,
        origin: Origin,
        home: &Path,
        picks: &[ImportPick],
        project: Option<&str>,
    ) -> Result<ImportResult, CoreError> {
        if picks.is_empty() {
            return Err(invalid("Pick at least one row to bring."));
        }
        let homes = homes(self, project).ok_or_else(|| invalid("The home folder is not known."))?;
        let own = home.to_path_buf();
        let mut chosen: Vec<(Found, Payload)> = crate::worktrees::blocking(move || {
            vw_import::scan_with_payloads(&vw_import::fs::Disk, &homes, &taken(&own))
        })
        .await?
        .into_iter()
        .filter(|(found, _)| picks.iter().any(|p| p.id == found.id))
        .collect();
        for pick in picks {
            let Some((found, _)) = chosen.iter().find(|(f, _)| f.id == pick.id) else {
                return Err(stale("A picked row is gone. Scan again."));
            };
            if found.fingerprint != pick.fingerprint {
                return Err(stale(&format!(
                    "{} changed since you looked. Scan again.",
                    found.name
                )));
            }
        }
        chosen.retain(|(found, _)| found.status != Status::CantBring);
        if chosen.is_empty() {
            return Err(invalid("None of the picked rows can be brought."));
        }
        let summary_text = confirmation(home, &chosen);
        self.ask_native(
            crate::command::requester(origin),
            METHOD,
            json!({"rows": chosen.len()}),
            summary_text,
        )
        .await?;
        let bring = now_ms().to_string();
        let backup = backup_dir(home, &bring)?;
        let mut record = Record::default();
        let mut summary = ImportSummary {
            bring: bring.clone(),
            brought: 0,
            changed: 0,
            left: Vec::new(),
        };
        std::fs::create_dir_all(&backup).map_err(|e| failed("The backup could not be made", e))?;
        for file in ["AGENTS.md", "config.toml"] {
            let path = home.join(file);
            if path.is_file() {
                std::fs::copy(&path, backup.join(file))
                    .map_err(|e| failed("The backup could not be made", e))?;
                record.backed_up.push(file.to_owned());
            } else {
                record.created_files.push(file.to_owned());
            }
        }
        for (found, payload) in chosen {
            match self.bring_one(home, &found, payload, &mut record).await {
                Ok(()) if found.status == Status::Changed => summary.changed += 1,
                Ok(()) => summary.brought += 1,
                Err(why) => summary.left.push(format!("{}: {why}", found.name)),
            }
        }
        record.at_ms = now_ms();
        record.summary = Some(summary.clone());
        self.shared
            .import_brings
            .0
            .lock_or_recover()
            .insert(bring, record);
        self.shared.emit(crate::event::CoreEvent::SecretsChanged {});
        Ok(ImportResult::Brought { summary })
    }

    /// Brings one row; the error text is shown in the summary.
    async fn bring_one(
        &self,
        home: &Path,
        found: &Found,
        payload: Payload,
        record: &mut Record,
    ) -> Result<(), String> {
        match payload {
            Payload::Instructions(text) => {
                let path = home.join("AGENTS.md");
                let before = std::fs::read_to_string(&path).unwrap_or_default();
                let after = format!(
                    "{}\n\n## Brought from {}\n\n{}\n",
                    before.trim_end(),
                    found.agent.label(),
                    text.trim_end()
                );
                vw_platform::replace_private(&path, after.trim_start().as_bytes())
                    .map_err(|e| e.to_string())
            }
            Payload::Skill(files) => {
                let folder = home.join("skills").join(&found.name);
                if !safe_relative(Path::new(&found.name)) || folder.exists() {
                    return Err("a skill with this name exists".to_owned());
                }
                for (relative, bytes) in &files {
                    if !safe_relative(relative) {
                        return Err("the folder holds an unexpected path".to_owned());
                    }
                    let target = folder.join(relative);
                    if let Some(parent) = target.parent() {
                        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                    }
                    std::fs::write(&target, bytes).map_err(|e| e.to_string())?;
                }
                record.skills.push(found.name.clone());
                Ok(())
            }
            Payload::Server(server) => self.bring_server(home, found, server, record).await,
            Payload::None => Err("nothing to bring".to_owned()),
        }
    }

    async fn bring_server(
        &self,
        home: &Path,
        found: &Found,
        server: Server,
        record: &mut Record,
    ) -> Result<(), String> {
        if found.kind != Kind::McpServer {
            return Err("not a server".to_owned());
        }
        // The name as Vibe itself reads it (the rule of a server added in Tools), so the config entry, the vault's
        // key and binding and the person's cards all carry the same plain name, never the other agent's raw text.
        let address = match &server {
            Server::Http { url, .. } => url.as_str(),
            Server::Stdio { .. } => "",
        };
        let name = crate::mcp_keys::server_name(Some(&found.name), address)
            .or_else(|_| crate::mcp_keys::server_name(None, address))
            .map_err(|e| e.message)?;
        match server {
            Server::Http { url, headers } if found.keys.len() == 1 => {
                let (header, value) = headers
                    .into_iter()
                    .find(|(k, _)| found.keys.contains(k))
                    .ok_or("the key is gone")?;
                let imported =
                    crate::mcp_keys::import_key_server(&self.shared, &name, &url, &header, value)
                        .await
                        .map_err(|e| e.message)?;
                record.key_servers.push(imported);
                Ok(())
            }
            server => {
                // The same address rule as a server added in Tools; the checked form is what is saved.
                let server = match server {
                    Server::Http { url, headers } => Server::Http {
                        url: vw_gate::check_mcp_server_url(&url).map_err(|r| r.to_string())?,
                        headers,
                    },
                    stdio @ Server::Stdio { .. } => stdio,
                };
                crate::mcp_keys_config::add_imported(home, &name, &server)
                    .map_err(|e| e.to_string())?;
                record.servers.push(name);
                Ok(())
            }
        }
    }
}

fn stale(message: &str) -> CoreError {
    let mut error = CoreError::new(ErrorCode::InvalidParams, message);
    error.server_code = Some("stale".to_owned());
    error
}

/// The confirmation: every file written and every key that goes to the vault (names only).
fn confirmation(home: &Path, chosen: &[(Found, Payload)]) -> String {
    let mut lines = vec![format!(
        "Bring {} item(s) from other coding agents into Vibe Work? Nothing in their folders changes.",
        chosen.len()
    )];
    for (found, _) in chosen {
        let line = match found.kind {
            Kind::Instructions => format!(
                "- {}: add to {}",
                vw_gate::dialog_value(&format!("{} instructions", found.agent.label()), 64),
                vw_gate::dialog_quote(&home.join("AGENTS.md").to_string_lossy())
            ),
            Kind::Skill => format!(
                "- skill {}: copy into {}",
                vw_gate::dialog_value(&found.name, 64),
                vw_gate::dialog_quote(&home.join("skills").join(&found.name).to_string_lossy())
            ),
            _ => format!(
                "- server {} from {}: {} (added to {}){}",
                vw_gate::dialog_value(&found.name, 64),
                vw_gate::dialog_quote(&found.source),
                vw_gate::dialog_value(found.detail.as_deref().unwrap_or(""), 2_000),
                vw_gate::dialog_quote(&home.join("config.toml").to_string_lossy()),
                if found.keys.is_empty() {
                    String::new()
                } else {
                    format!(
                        "; key {} goes to your vault",
                        vw_gate::dialog_value(&found.keys.join(", "), 64)
                    )
                }
            ),
        };
        lines.push(line);
    }
    lines.join("\n")
}

/// The brings of this run, by bring id: the core's own record of what each changed, the only input of Undo.
#[derive(Debug, Default)]
pub(crate) struct Brings(std::sync::Mutex<std::collections::HashMap<String, Record>>);

/// What one bring changed.
#[derive(Debug, Default, Clone)]
pub(crate) struct Record {
    at_ms: u64,
    backed_up: Vec<String>,
    created_files: Vec<String>,
    skills: Vec<String>,
    servers: Vec<String>,
    key_servers: Vec<crate::mcp_keys::ImportedKey>,
    /// What the bring reported, shown again with its Undo.
    summary: Option<ImportSummary>,
}

/// The newest bring of this run that can still be undone (within 24 hours).
fn last_bring(core: &Core) -> Option<ImportSummary> {
    let now = now_ms();
    core.shared
        .import_brings
        .0
        .lock_or_recover()
        .values()
        .filter(|r| now.saturating_sub(r.at_ms) <= UNDO_MS)
        .max_by_key(|r| r.at_ms)
        .and_then(|r| r.summary.clone())
}

async fn import_undo(core: &Core, home: &Path, bring: &str) -> Result<(), CoreError> {
    let backup = backup_dir(home, bring)?;
    let record = core
        .shared
        .import_brings
        .0
        .lock_or_recover()
        .get(bring)
        .cloned()
        .ok_or_else(|| CoreError::new(ErrorCode::NotFound, "No such bring to undo."))?;
    if now_ms().saturating_sub(record.at_ms) > UNDO_MS {
        return Err(invalid("A bring can be undone for 24 hours only."));
    }
    for imported in &record.key_servers {
        crate::mcp_keys::remove_imported(&core.shared, imported).await?;
    }
    for name in &record.skills {
        if safe_relative(Path::new(name)) {
            let folder = home.join("skills").join(name);
            if folder.is_dir() {
                std::fs::remove_dir_all(&folder)
                    .map_err(|e| failed("A skill could not be removed", e))?;
            }
        }
    }
    for file in &record.backed_up {
        let saved =
            std::fs::read(backup.join(file)).map_err(|e| failed("The backup is unreadable", e))?;
        vw_platform::replace_private(&home.join(file), &saved)
            .map_err(|e| failed("A file could not be put back", e))?;
    }
    for file in &record.created_files {
        let path = home.join(file);
        if path.is_file() {
            std::fs::remove_file(&path).map_err(|e| failed("A file could not be removed", e))?;
        }
    }
    std::fs::remove_dir_all(&backup).map_err(|e| failed("The backup could not be removed", e))?;
    core.shared.import_brings.0.lock_or_recover().remove(bring);
    core.shared.emit(crate::event::CoreEvent::SecretsChanged {});
    Ok(())
}
