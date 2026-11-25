//! Adding and changing command-started (stdio) MCP servers from Tools, under Developer permissions.
//!
//! Depends on: `command_secret`, `command_servers_config`, `command_pause`, `connector_consent` (the dialog text),
//! `vw_platform::command_resolve` (program resolution), `connectors` (refresh, view). Called by: `connectors`.
//! Invariants:
//! - **Only the person's window**: `Origin::DebugApi`, and window commands sent while the Debug API drives the window,
//!   are refused (`debug_api_acts`); so is everything while Developer permissions is off. The gate keeps refusing a
//!   command server on the agent's and Vibe's own add paths (`vw_gate` `mcp_url`).
//! - Everything is checked before the native dialog, so the person is never asked about something that would be
//!   refused: the program is resolved to an absolute path (never run), the working folder must exist, a value that
//!   looks like a key is refused (`command_secret`), names are checked.
//! - The dialog shows what will be saved; nothing is written before the person says yes, and Developer permissions
//!   is checked again after the answer.
//! - The program is started only by Vibe's normal connect, after the entry is saved.

use serde_json::{Value, json};
use tracing::info;

use crate::command::Origin;
use crate::command_secret::{HIDDEN_ARGUMENT, looks_like_secret};
use crate::command_servers_config::{
    CommandEntry, add_command_entry, command_entry_checked, replace_command_entry,
};
use crate::connector_consent::{CommandDialog, command_server_summary};
use crate::connectors_view::{CommandEnvInput, ConnectorsRequest, ConnectorsResult};
use crate::core::Core;
use crate::error::{CoreError, ErrorCode};

const METHOD: &str = "vibe-work/mcpCommandServer";
const MAX_ARGS: usize = 128;
const MAX_ENV: usize = 64;
const MAX_TEXT: usize = 4096;

/// The settings as the person typed them.
pub(crate) struct CommandDraft {
    pub name: Option<String>,
    pub command: String,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    pub env: Vec<CommandEnvInput>,
}

fn invalid(message: impl Into<String>) -> CoreError {
    CoreError::new(ErrorCode::InvalidParams, message)
}

fn refused(message: &str) -> CoreError {
    CoreError::new(ErrorCode::Denied, message)
}

/// True only for a Vibe method that can clear a configured server's `disabled` flag.
pub(crate) fn is_server_toggle(method: &str, params: &Value) -> bool {
    matches!(method, "mcp/toggle" | "mcp_catalog/toggle")
        && params.get("source").and_then(Value::as_str) == Some("server")
}

/// Variables that would change how the app's own programs load code or find their settings.
fn env_name_refusal(name: &str) -> Option<&'static str> {
    let ok_shape = !name.is_empty()
        && name.len() <= 128
        && name
            .chars()
            .enumerate()
            .all(|(i, c)| c == '_' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit()));
    if !ok_shape {
        return Some(
            "a variable name has letters, digits and underscores only, and does not start with a digit",
        );
    }
    let upper = name.to_ascii_uppercase();
    if upper.starts_with("LD_")
        || upper.starts_with("DYLD_")
        || upper.starts_with("VW_")
        || upper.starts_with("VIBE_")
        || matches!(upper.as_str(), "BASH_ENV" | "ENV")
    {
        return Some(
            "that variable changes how programs load code or how Vibe Work runs, and is not allowed here",
        );
    }
    None
}

fn clean_text(label: &str, text: &str) -> Result<(), CoreError> {
    if text.chars().count() > MAX_TEXT
        || text
            .chars()
            .any(|c| c == '\0' || (c.is_control() && c != '\t'))
    {
        return Err(invalid(format!(
            "{label} is too long or has a control character"
        )));
    }
    Ok(())
}

/// What the dialog shows and what is saved.
struct Prepared {
    name: String,
    entry: CommandEntry,
}

/// The settings after the checks: arguments, variables (with `keepExisting` resolved) and the working folder.
struct Checked {
    args: Vec<String>,
    env: Vec<(String, String)>,
    cwd: Option<String>,
}

/// The plain settings, checked: arguments, variables (with `keepExisting` resolved from `existing`), working folder.
fn checked(draft: &CommandDraft, existing: Option<&CommandEntry>) -> Result<Checked, CoreError> {
    if draft.args.len() > MAX_ARGS || draft.env.len() > MAX_ENV {
        return Err(invalid(format!(
            "at most {MAX_ARGS} arguments and {MAX_ENV} variables"
        )));
    }
    for arg in &draft.args {
        clean_text("an argument", arg)?;
        if arg == HIDDEN_ARGUMENT {
            return Err(invalid(
                "an argument is hidden because it looks like a key: type the arguments again without the key",
            ));
        }
        if looks_like_secret(None, arg) {
            return Err(CoreError::new(
                ErrorCode::InvalidParams,
                vw_vocabulary::texts::COMMAND_SERVER_SECRET_REFUSED,
            ));
        }
    }
    let mut env: Vec<(String, String)> = Vec::new();
    for item in &draft.env {
        let name = item.name.trim();
        if let Some(why) = env_name_refusal(name) {
            return Err(invalid(format!(
                "variable {}: {why}",
                vw_gate::dialog_value(name, 40)
            )));
        }
        if env.iter().any(|(n, _)| n == name) {
            return Err(invalid(format!(
                "variable {} is listed twice",
                vw_gate::dialog_value(name, 40)
            )));
        }
        let value = match (&item.value, item.keep_existing == Some(true)) {
            (Some(value), _) => {
                clean_text("a variable value", value)?;
                if looks_like_secret(Some(name), value) {
                    return Err(CoreError::new(
                        ErrorCode::InvalidParams,
                        vw_vocabulary::texts::COMMAND_SERVER_SECRET_REFUSED,
                    ));
                }
                value.clone()
            }
            (None, true) => existing
                .and_then(|e| e.env.iter().find(|(n, _)| n == name))
                .map(|(_, v)| v.clone())
                .ok_or_else(|| {
                    invalid(format!(
                        "there is no saved value for {}",
                        vw_gate::dialog_value(name, 40)
                    ))
                })?,
            (None, false) => String::new(),
        };
        env.push((name.to_owned(), value));
    }
    let cwd = match draft
        .cwd
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty())
    {
        None => None,
        Some(folder) => {
            clean_text("the working folder", folder)?;
            if !std::path::Path::new(folder).is_absolute() || !std::path::Path::new(folder).is_dir()
            {
                return Err(invalid(format!(
                    "the working folder {} does not exist: choose an existing folder or leave it empty",
                    vw_gate::dialog_value(folder, 80)
                )));
            }
            Some(folder.to_owned())
        }
    };
    Ok(Checked {
        args: draft.args.clone(),
        env,
        cwd,
    })
}

/// A server name from the program's file name (`C:\tools\mcp-fixture.exe` -> `mcp-fixture`).
fn derived_name(program: &str) -> String {
    let leaf = program.rsplit(['/', '\\']).next().unwrap_or(program);
    let lower = leaf.to_ascii_lowercase();
    let stem = [".exe", ".cmd", ".bat", ".com"]
        .iter()
        .find(|ext| lower.ends_with(*ext))
        .map_or(leaf, |ext| &leaf[..leaf.len() - ext.len()]);
    crate::mcp_keys::server_name(Some(stem), "").unwrap_or_else(|_| "command".to_owned())
}

impl Core {
    /// The rule of the module docs: the person's window, with Developer permissions on.
    fn command_server_gate(&self, origin: Origin) -> Result<(), CoreError> {
        if crate::command_origin::debug_api_acts(&self.shared, origin) {
            return Err(refused(vw_vocabulary::texts::COMMAND_SERVER_WINDOW_ONLY));
        }
        if !self.shared.developer_permissions.enabled() {
            return Err(refused(
                vw_vocabulary::texts::COMMAND_SERVER_NEEDS_DEVELOPER_ERROR,
            ));
        }
        Ok(())
    }

    fn forwarded_toggle_entry(&self, name: &str) -> Result<Option<CommandEntry>, CoreError> {
        command_entry_checked(&self.shared.instructions.own_home, name)
            .map_err(|_| refused(vw_vocabulary::texts::COMMAND_SERVER_INSPECT_FAILED))
    }

    /// Ask natively for the exact saved command before a raw or typed toggle enables it.
    pub(crate) async fn prepare_forwarded_toggle(
        &self,
        origin: Origin,
        method: &str,
        params: &Value,
    ) -> Result<Option<(CommandEntry, u64)>, CoreError> {
        if !is_server_toggle(method, params)
            || params.get("disabled").and_then(Value::as_bool) != Some(false)
        {
            return Ok(None);
        }
        // A bad shape must not show a dialog that the gate will refuse after the answer.
        self.shared
            .gate
            .check(method, params, None)
            .map_err(|error| CoreError::from_gate(method, &error))?;
        let Some(name) = params.get("name").and_then(Value::as_str) else {
            return Ok(None);
        };
        let Some(entry) = self.forwarded_toggle_entry(name)? else {
            return Ok(None);
        };
        self.command_server_gate(origin)?;
        let command = entry.command.clone();
        let cwd = entry.cwd.clone();
        let program = crate::worktrees::blocking(move || {
            vw_platform::command_resolve::resolve_here(&command, cwd.as_deref())
        })
        .await?
        .map_err(|error| invalid(error.to_string()))?;
        vw_platform::command_resolve::check_launch(
            vw_platform::command_resolve::OsKind::current(),
            &program,
            &entry.args,
        )
        .map_err(|error| invalid(error.to_string()))?;
        let revision = self.shared.developer_permissions.revision();
        let shown_env: Vec<(String, Option<String>)> = entry
            .env
            .iter()
            .map(|(key, value)| {
                (
                    key.clone(),
                    (!looks_like_secret(Some(key), value)).then(|| value.clone()),
                )
            })
            .collect();
        let summary = command_server_summary(&CommandDialog {
            change: false,
            enable: true,
            import: false,
            name,
            program: &program,
            args: &entry.args,
            cwd: entry.cwd.as_deref(),
            env: &shown_env,
        });
        self.ask_native(
            crate::command::requester(origin),
            METHOD,
            json!({"method": method, "name": name, "disabled": false}),
            summary,
        )
        .await?;
        Ok(Some((entry, revision)))
    }

    /// Under the transition lock, re-read the target and switch after the native answer and before Vibe can send.
    pub(crate) fn check_forwarded_toggle(
        &self,
        origin: Origin,
        params: &Value,
        approved: Option<&(CommandEntry, u64)>,
    ) -> Result<bool, CoreError> {
        let Some(name) = params.get("name").and_then(Value::as_str) else {
            return Ok(false);
        };
        let target = self.forwarded_toggle_entry(name)?;
        let enabling = params.get("disabled").and_then(Value::as_bool) == Some(false);
        if enabling && target.is_some() {
            self.command_server_gate(origin)?;
        }
        if let Some((entry, revision)) = approved {
            if target.as_ref() != Some(entry)
                || self.shared.developer_permissions.revision() != *revision
            {
                return Err(refused(vw_vocabulary::texts::COMMAND_SERVER_CHANGED));
            }
        } else if enabling && target.is_some() {
            return Err(refused(vw_vocabulary::texts::COMMAND_SERVER_CHANGED));
        }
        Ok(target.is_some())
    }

    /// `addCommandServer` and `updateCommandServer` (the dispatch of `connectors`).
    pub(crate) async fn command_server_request(
        &self,
        origin: Origin,
        request: ConnectorsRequest,
    ) -> Result<ConnectorsResult, CoreError> {
        match request {
            ConnectorsRequest::AddCommandServer {
                name,
                command,
                args,
                cwd,
                env,
            } => {
                let draft = CommandDraft {
                    name,
                    command,
                    args,
                    cwd,
                    env,
                };
                self.command_server_change(origin, false, draft).await
            }
            ConnectorsRequest::UpdateCommandServer {
                name,
                command,
                args,
                cwd,
                env,
            } => {
                if name.trim().is_empty() || name.chars().any(char::is_control) {
                    return Err(invalid(
                        "the name must not be empty or hold control characters",
                    ));
                }
                let draft = CommandDraft {
                    name: Some(name),
                    command,
                    args,
                    cwd,
                    env,
                };
                self.command_server_change(origin, true, draft).await
            }
            other => Err(invalid(format!("not a command server request: {other:?}"))),
        }
    }

    /// Everything the dialog will show, checked before it is shown: the settings, the resolved program, the name.
    async fn prepare_command(
        &self,
        update: bool,
        draft: &CommandDraft,
    ) -> Result<Prepared, CoreError> {
        let home = self.shared.instructions.own_home.clone();
        let existing = match (update, draft.name.as_deref()) {
            (true, Some(name)) => Some(
                command_entry_checked(&home, name)
                    .map_err(|_| refused(vw_vocabulary::texts::COMMAND_SERVER_INSPECT_FAILED))?
                    .ok_or_else(|| invalid("no command server with that name is configured"))?,
            ),
            _ => None,
        };
        let Checked { args, env, cwd } = checked(draft, existing.as_ref())?;
        let (command_text, cwd_text) = (draft.command.clone(), cwd.clone());
        let program = crate::worktrees::blocking(move || {
            vw_platform::command_resolve::resolve_here(&command_text, cwd_text.as_deref())
        })
        .await?
        .map_err(|e| invalid(e.to_string()))?;
        vw_platform::command_resolve::check_launch(
            vw_platform::command_resolve::OsKind::current(),
            &program,
            &args,
        )
        .map_err(|e| invalid(e.to_string()))?;
        let typed = draft
            .name
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty());
        let name = match typed {
            Some(name) if update => name.to_owned(),
            Some(name) => crate::mcp_keys::server_name(Some(name), "")?,
            None => derived_name(&program),
        };
        if !update
            && crate::connectors_config::configured_servers(&home)
                .iter()
                .any(|s| s.name == name)
        {
            return Err(invalid(format!(
                "an MCP server named {} is already configured: remove it first or type another name",
                vw_gate::dialog_value(&name, 64)
            )));
        }
        Ok(Prepared {
            name,
            entry: CommandEntry {
                command: program,
                args,
                cwd,
                env,
            },
        })
    }

    /// Adds (`update` = false) or changes (`update` = true) a command server after the person's native answer.
    async fn command_server_change(
        &self,
        origin: Origin,
        update: bool,
        draft: CommandDraft,
    ) -> Result<ConnectorsResult, CoreError> {
        self.command_server_gate(origin)?;
        let Prepared { name, entry } = self.prepare_command(update, &draft).await?;
        let shown: Vec<(String, Option<String>)> = entry
            .env
            .iter()
            .map(|(n, v)| {
                (
                    n.clone(),
                    (!looks_like_secret(Some(n), v)).then(|| v.clone()),
                )
            })
            .collect();
        let summary = command_server_summary(&CommandDialog {
            change: update,
            enable: false,
            import: false,
            name: &name,
            program: &entry.command,
            args: &entry.args,
            cwd: entry.cwd.as_deref(),
            env: &shown,
        });
        self.ask_native(
            crate::command::requester(origin),
            METHOD,
            json!({"server": name, "program": entry.command, "change": update}),
            summary,
        )
        .await?;
        // The answer took time: Developer permissions may have been turned off meanwhile.
        let mutation = self.shared.developer_permissions.mutation_lock().await;
        self.command_server_gate(origin)?;
        #[cfg(test)]
        save_test_seam::before_write(&name).await;
        let home = self.shared.instructions.own_home.clone();
        let saved = crate::worktrees::blocking({
            let name = name.clone();
            move || {
                if update {
                    replace_command_entry(&home, &name, &entry)
                } else {
                    add_command_entry(&home, &name, &entry)
                }
            }
        })
        .await?;
        saved.map_err(|error| {
            use crate::mcp_keys_config::ConfigEditError as E;
            let code = match error {
                E::NameTaken | E::NotACommandServer => ErrorCode::InvalidParams,
                _ => ErrorCode::Internal,
            };
            CoreError::new(code, error.to_string())
        })?;
        #[cfg(test)]
        save_test_seam::after_write(&name).await;
        self.shared.connectors.mark_removed(&name, false);
        crate::connectors::refresh_servers_locked(&self.shared).await;
        drop(mutation);
        info!(update, "connectors: command MCP server saved");
        let view = self.connectors_view(origin, None).await;
        Ok(if update {
            ConnectorsResult::View { view }
        } else {
            ConnectorsResult::ServerAdded {
                name,
                created: true,
                view,
            }
        })
    }

    /// A removed server is no longer in the app's paused list.
    pub(crate) fn forget_paused(&self, name: &str) {
        crate::command_pause::forget(self.shared.developer_permissions.settings_path(), name);
    }

    /// A typed command toggle preflights the same rule as the forwarded call. The forwarder owns the
    /// final check, native answer, send lock and paused-list update.
    pub(crate) fn check_server_off(
        &self,
        origin: Origin,
        name: &str,
        off: bool,
    ) -> Result<(), CoreError> {
        let home = &self.shared.instructions.own_home;
        if command_entry_checked(home, name)
            .map_err(|_| refused(vw_vocabulary::texts::COMMAND_SERVER_INSPECT_FAILED))?
            .is_none()
        {
            return Ok(());
        }
        if !off {
            self.command_server_gate(origin)?;
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod save_test_seam {
    use std::sync::{Arc, Mutex, OnceLock};

    pub(crate) struct Hold {
        pub name: String,
        pub reached: tokio::sync::Semaphore,
        pub release: tokio::sync::Semaphore,
        pub wrote: tokio::sync::Semaphore,
        pub finish: tokio::sync::Semaphore,
    }

    static HOLD: OnceLock<Mutex<Option<Arc<Hold>>>> = OnceLock::new();

    pub(crate) fn set(hold: Option<Arc<Hold>>) {
        *HOLD
            .get_or_init(|| Mutex::new(None))
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = hold;
    }

    pub(crate) async fn before_write(name: &str) {
        let hold = HOLD
            .get_or_init(|| Mutex::new(None))
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if let Some(hold) = hold.filter(|hold| hold.name == name) {
            hold.reached.add_permits(1);
            hold.release.acquire().await.unwrap().forget();
        }
    }

    pub(crate) async fn after_write(name: &str) {
        let hold = HOLD
            .get_or_init(|| Mutex::new(None))
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if let Some(hold) = hold.filter(|hold| hold.name == name) {
            hold.wrote.add_permits(1);
            hold.finish.acquire().await.unwrap().forget();
        }
    }
}
