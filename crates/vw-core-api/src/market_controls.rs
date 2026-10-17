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

fn quoted_label(name: &str) -> String {
    let mut label = String::new();
    let mut length = 0;
    let mut chars = name.chars().peekable();
    while let Some(ch) = chars.next() {
        let width = vw_gate::dialog_quote(&ch.to_string()).chars().count() - 2;
        let limit = if chars.peek().is_some() { 79 } else { 80 };
        if length + width > limit {
            label.push('…');
            break;
        }
        label.push(ch);
        length += width;
    }
    vw_gate::dialog_quote(&label)
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
            .find(|p| p["path"].as_str() == Some(path.as_str()))
            .ok_or_else(|| {
                refuse("Vibe no longer discovers this plugin. Read the Tools page again.")
            })?;
        let name = row["name"].as_str().unwrap_or("plugin");
        self.confirm_plugin_change(origin, op, &path, name).await?;
        let held = self.shared.plugin_approvals.mutations.lock().await;
        // A dialog can remain open while an agent edits the tree. Check ownership again after it closes.
        if !self.plugin_owned(folder, project.as_deref(), off) {
            return Err(refuse(
                "The plugin folder changed while the request was open. Nothing was changed.",
            ));
        }
        let user_plugin = folder
            .parent()
            .is_some_and(|p| p == home.join(if off { "plugins-off" } else { "plugins" }));
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
        self.reload_sessions().await;
        if op == "removePlugin"
            && folder.starts_with(home)
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
        let name = quoted_label(name);
        if op == "turnOnPlugin" {
            self.ask_native(
                crate::command::requester(origin), "vw/plugins/turnOn", json!({"path":path}),
                format!("Turn on “{}” from the folder {}? Its MCP servers will run in your new chats, with your permissions.",
                    name.strip_prefix('"').and_then(|s| s.strip_suffix('"')).unwrap_or(&name),
                    vw_gate::dialog_quote(path)),
            ).await?;
        }
        if op == "removePlugin" {
            let quote = vw_gate::dialog_quote;
            self.ask_native(
                crate::command::requester(origin),
                "vw/plugins/remove",
                json!({"path":path}),
                format!(
                    "Remove plugin {} and delete its folder {}?",
                    name,
                    quote(path)
                ),
            )
            .await?;
        }
        Ok(())
    }
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
    std::fs::rename(folder, target).map_err(|e| {
        CoreError::new(
            ErrorCode::Internal,
            format!("The plugin could not be moved: {e}"),
        )
    })?;

    Ok(())
}
