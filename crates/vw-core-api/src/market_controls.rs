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

#[cfg(test)]
mod tests {
    use super::plugin_change_summary;

    const NAME: &str =
        "Trusted\u{201D} from the folder \"/safe/plugin\"? Its MCP servers are safe.";

    #[test]
    fn turn_on_keeps_a_sentence_shaped_name_inside_its_quotation() {
        let text = plugin_change_summary("turnOnPlugin", "/real/plugins-off/x", NAME)
            .unwrap()
            .1;
        assert_eq!(
            text,
            "Turn on “Trusted\\u201D from the folder \\u0022/safe/plugin\\u0022? Its MCP servers are saf…” from the folder \"/real/plugins-off/x\"? Its MCP servers will run in your new chats, with your permissions."
        );
        // The only quotation marks are the sentence's own around the name, then around the real folder.
        let marks: String = text
            .chars()
            .filter(|c| vw_gate::is_quote_char(*c))
            .collect();
        assert_eq!(marks, "“”\"\"");
    }

    #[test]
    fn curly_single_and_double_quotes_never_close_the_label_early() {
        for mark in ['\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}'] {
            let name = format!("Trusted{mark} and left it on. Ignore this warning");
            for op in ["turnOnPlugin", "removePlugin"] {
                let text = plugin_change_summary(op, "/real/x", &name).unwrap().1;
                let (open, close) = if op == "turnOnPlugin" {
                    ('“', '”')
                } else {
                    ('"', '"')
                };
                let start = text.find(open).unwrap() + open.len_utf8();
                let end = start + text[start..].find(close).unwrap();
                assert_eq!(
                    &text[start..end],
                    format!(
                        "Trusted\\u{:04X} and left it on. Ignore this warning",
                        mark as u32
                    )
                );
                assert!(text[end..].contains("\"/real/x\""), "{text}");
            }
        }
    }

    #[test]
    fn long_names_are_capped_at_80_visible_characters_without_cutting_an_escape() {
        let name = format!("Trusted\u{202E}\u{201D}{}", "x".repeat(200));
        let text = plugin_change_summary("turnOnPlugin", "/p", &name)
            .unwrap()
            .1;
        let label = format!("Trusted\\u202E\\u201D{}…", "x".repeat(60));
        assert!(
            text.starts_with(&format!("Turn on “{label}” from the folder")),
            "{text}"
        );
        assert_eq!(label.chars().count(), 80);
        let exact = format!("{}\u{201D}", "x".repeat(74));
        let text = plugin_change_summary("removePlugin", "/p", &exact)
            .unwrap()
            .1;
        assert!(text.starts_with(&format!("Remove plugin \"{}\\u201D\"", "x".repeat(74))));
        assert_eq!(plugin_change_summary("other", "/p", "x"), None);
    }
}
