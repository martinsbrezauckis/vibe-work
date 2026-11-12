//! Owned plugin folder changes for Tools. Depends on Vibe discovery, path ownership and native dialogs;
//! called by `market`. Moves preserve all plugin bytes and never overwrite an existing destination.

use std::path::Path;

use serde_json::json;

use crate::command::Origin;
use crate::core::Core;
use crate::error::{CoreError, ErrorCode};
use crate::market_view::MarketResult;

fn refuse(message: &str) -> CoreError {
    CoreError::new(ErrorCode::InvalidParams, message)
}

/// A manifest name as shown between quotation marks in a plugin confirmation: visibly escaped by
/// [`vw_gate::dialog_label`] and at most 80 visible characters, where an escape counts at its written width
/// and is never cut; a shortened name ends with `…`.
fn plugin_label(name: &str) -> String {
    let mut label = String::new();
    let mut length = 0;
    let mut chars = name.chars().peekable();
    while let Some(ch) = chars.next() {
        let token = vw_gate::dialog_label(&ch.to_string());
        let width = token.chars().count();
        let limit = if chars.peek().is_some() { 79 } else { 80 };
        if length + width > limit {
            label.push('…');
            break;
        }
        label.push_str(&token);
        length += width;
    }
    label
}

/// The native confirmation method and sentence for a plugin folder change, or `None` when `op` needs no
/// confirmation. The manifest name stays inside its own quotation marks, so the fixed text and the real
/// folder that follow it are always the sentence's own.
fn plugin_change_summary(op: &str, path: &str, name: &str) -> Option<(&'static str, String)> {
    let label = plugin_label(name);
    let folder = vw_gate::dialog_quote(path);
    match op {
        "turnOnPlugin" => Some((
            "vw/plugins/turnOn",
            format!(
                "Turn on “{label}” from the folder {folder}? Its MCP servers will run in your new chats, with your permissions."
            ),
        )),
        "removePlugin" => Some((
            "vw/plugins/remove",
            format!("Remove plugin \"{label}\" and delete its folder {folder}?"),
        )),
        _ => None,
    }
}

impl Core {
    pub(crate) async fn change_plugin(
        &self,
        origin: Origin,
        op: &str,
        path: String,
        session_id: Option<String>,
        cwd: Option<String>,
    ) -> Result<MarketResult, CoreError> {
        let project = self.plugin_project(session_id.as_deref(), cwd.as_deref());
        let folder = Path::new(&path);
        let home = &self.shared.instructions.own_home;
        let off = folder
            .parent()
            .is_some_and(|p| p.file_name().is_some_and(|n| n == "plugins-off"));
        if !self.plugin_owned(folder, project.as_deref(), off) {
            return Err(refuse(
                "This plugin is managed elsewhere. Only plugins in Vibe Work's own home or the opened project can be changed.",
            ));
        }
        if (op == "turnOffPlugin" && off) || (op == "turnOnPlugin" && !off) {
            return Err(refuse(
                "The plugin's on/off state changed. Read the Tools page again.",
            ));
        }
        // Re-resolve before changing anything: a client path or a folder name is never evidence of a plugin.
        let discovered = self
            .discover_plugins(project.as_deref(), session_id.as_deref())
            .await
            .map_err(|message| CoreError::new(ErrorCode::Internal, message))?;
        let row = discovered["plugins"]
            .as_array()
            .into_iter()
            .flatten()
            // Compared as paths: on Windows `C:\a\b` and `C:/a/b` (as an agent on the Debug API may write it) are
            // the same folder.
            .find(|p| {
                p["path"]
                    .as_str()
                    .is_some_and(|found| Path::new(found) == folder)
            })
            .ok_or_else(|| {
                refuse("Vibe no longer discovers this plugin. Read the Tools page again.")
            })?;
        let name = row["name"].as_str().unwrap_or("plugin");
        if op == "turnOnPlugin" && row["readable"] == false {
            return Err(refuse(
                "This plugin is not readable. Check its issues in Tools before turning it on.",
            ));
        }
        self.confirm_plugin_change(origin, op, &path, name).await?;
        let held = self.shared.plugin_approvals.mutations.lock().await;
        // A dialog can remain open while an agent edits the tree. Check ownership again after it closes.
        if !self.plugin_owned(folder, project.as_deref(), off) {
            return Err(refuse(
                "The plugin folder changed while the request was open. Nothing was changed.",
            ));
        }
        let user_plugin = home_plugin(folder, home, off);
        if user_plugin {
            // Revoke before changing bytes. A failed move or delete leaves the plugin off on the next sweep.
            self.shared.plugin_approvals.set(folder, None)?;
        }
        let receipt = crate::market_local::receipt(folder);
        let mut notice = None;
        if op == "removePlugin" {
            std::fs::remove_dir_all(folder).map_err(|e| {
                CoreError::new(
                    ErrorCode::Internal,
                    format!("The plugin folder could not be removed: {e}"),
                )
            })?;
        } else {
            move_plugin(folder, off)?;
            if user_plugin && op == "turnOnPlugin" {
                let target = home.join("plugins").join(
                    folder
                        .file_name()
                        .ok_or_else(|| refuse("The plugin folder has no name."))?,
                );
                if let Err(error) = self.shared.plugin_approvals.set(&target, Some(name)) {
                    move_plugin(&target, false)?;
                    return Err(error);
                }
            }
        }
        drop(held);
        // Its tools leave the chats: their "Allow for this session" answers end (a plugin turned on again asks).
        if op == "removePlugin" || op == "turnOffPlugin" {
            crate::ask::grants::plugin_changed(&self.shared);
        }
        self.reload_sessions().await;
        if op == "removePlugin"
            && user_plugin
            && let Some(receipt) = receipt
        {
            notice = self
                .store()
                .remove_data(&receipt.entry_id)
                .err()
                .map(|_| "Removed; the plugin's data folder could not be deleted.".to_owned());
        }
        Ok(MarketResult::View {
            view: Box::new(self.market_view(session_id, cwd).await),
            notice,
        })
    }

    async fn confirm_plugin_change(
        &self,
        origin: Origin,
        op: &str,
        path: &str,
        name: &str,
    ) -> Result<(), CoreError> {
        if let Some((method, summary)) = plugin_change_summary(op, path, name) {
            self.ask_native(
                crate::command::requester(origin),
                method,
                json!({"path":path}),
                summary,
            )
            .await?;
        }
        Ok(())
    }
}

/// `folder` lies directly in the home's `plugins` folder (`plugins-off` when `off`). Compared resolved as well:
/// Vibe reports plugin folders in their resolved spelling, while the home keeps the one the platform gave, which can
/// differ (a profile moved behind a link, another case on Windows).
fn home_plugin(folder: &Path, home: &Path, off: bool) -> bool {
    let root = home.join(if off { "plugins-off" } else { "plugins" });
    folder.parent().is_some_and(|parent| {
        parent == root
            || matches!(
                (parent.canonicalize(), root.canonicalize()),
                (Ok(parent), Ok(root)) if parent == root
            )
    })
}

pub(crate) fn move_plugin(folder: &Path, off: bool) -> Result<(), CoreError> {
    let parent = folder
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| refuse("The plugin root is unavailable."))?;
    let target_root = parent.join(if off { "plugins" } else { "plugins-off" });
    if std::fs::symlink_metadata(&target_root)
        .is_ok_and(|m| m.file_type().is_symlink() || !m.is_dir())
    {
        return Err(refuse(
            "The destination plugins folder is linked or is not a folder. Nothing was moved.",
        ));
    }
    std::fs::create_dir_all(&target_root).map_err(|e| {
        CoreError::new(
            ErrorCode::Internal,
            format!("The plugins folder could not be created: {e}"),
        )
    })?;
    let target = target_root.join(
        folder
            .file_name()
            .ok_or_else(|| refuse("The plugin folder has no name."))?,
    );
    if std::fs::symlink_metadata(&target).is_ok() {
        return Err(refuse(
            "A plugin folder already exists at the destination. Nothing was overwritten.",
        ));
    }
    rename_settled(
        folder,
        &target,
        &mut |from, to| std::fs::rename(from, to),
        &mut std::thread::sleep,
        cfg!(windows),
    )
}

/// Pauses between attempts while Windows reports the plugin folder busy: about 4.5 s in all. Another process reading
/// files in the folder blocks the move: antivirus or Explorer on a newly copied folder, or a plugin read. (The Tools
/// board's own reader, `discover_plugins`, no longer reads folders in `plugins/` its sweep has not kept.) Measured on
/// the Windows test machine: about 150 ms of reads in one Chrome Control folder; the next try 260 ms later passed.
const BUSY_PAUSES_MS: [u64; 8] = [50, 100, 200, 400, 800, 1000, 1000, 1000];

/// Windows refuses to rename a folder while any process has a file or subfolder inside it open, whatever that
/// handle's sharing (`ERROR_ACCESS_DENIED`, os error 5), or has the folder itself open without delete sharing
/// (`ERROR_SHARING_VIOLATION`, 32); both pass once the other process lets go. Off Windows the same numbers mean
/// EIO/EPIPE and are never retried.
fn busy(error: &std::io::Error, windows: bool) -> bool {
    windows && matches!(error.raw_os_error(), Some(5 | 32))
}

/// Moves `from` to `to`, waiting out a briefly busy folder on Windows (see [`BUSY_PAUSES_MS`]); the move is one
/// rename, so the folder is either moved whole or left where it was. The destination is checked again before
/// each retry: an existing one is never overwritten. Blocks the calling thread for at most the pauses' sum, only
/// while the folder is busy. `rename` and `pause` are the seams the tests inject.
fn rename_settled(
    from: &Path,
    to: &Path,
    rename: &mut dyn FnMut(&Path, &Path) -> std::io::Result<()>,
    pause: &mut dyn FnMut(std::time::Duration),
    windows: bool,
) -> Result<(), CoreError> {
    let mut pauses = BUSY_PAUSES_MS.iter();
    loop {
        let error = match rename(from, to) {
            Ok(()) => return Ok(()),
            Err(error) => error,
        };
        if !busy(&error, windows) {
            return Err(CoreError::new(
                ErrorCode::Internal,
                format!("The plugin could not be moved: {error}"),
            ));
        }
        let Some(ms) = pauses.next() else {
            let name = from.file_name().unwrap_or_default().to_string_lossy();
            return Err(CoreError::new(
                ErrorCode::Internal,
                format!(
                    "Windows is still using a file in the plugin folder \"{name}\" (often antivirus checking newly copied files, or a program that has the folder open). Nothing was moved. Wait a moment and try again."
                ),
            ));
        };
        pause(std::time::Duration::from_millis(*ms));
        if std::fs::symlink_metadata(to).is_ok() {
            return Err(refuse(
                "A plugin folder already exists at the destination. Nothing was overwritten.",
            ));
        }
    }
}

#[cfg(test)]
#[path = "market_controls_unit_tests.rs"]
mod tests;
