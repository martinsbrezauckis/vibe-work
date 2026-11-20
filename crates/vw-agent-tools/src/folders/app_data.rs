//! Reserves app data outside the readable Vibe home, except for the current chat's scratch folder, and the private
//! vault beside the app data.
//! Uses the folder guard's kernel-order path resolution; called alongside its existing control-file and copy checks.
use super::{Access, Layout, Permits, resolve, shown};
use std::path::{Component, Path, PathBuf};

/// The folder beside the app data that holds the private vault on every OS. It must name the same folder as
/// `vw_vault::private_location::private_vault_dir`'s parent (a core test compares the two; this binary does not link
/// the vault).
pub const PRIVATE_VAULT_CONTAINER: &str = "Vibe Work Vault";

/// `<parent of app_data>/Vibe Work Vault`, or `None` when `app_data` has no parent. No filesystem access.
#[must_use]
pub fn private_vault_container(app_data: &Path) -> Option<PathBuf> {
    app_data
        .parent()
        .filter(|parent| *parent != app_data)
        .map(|parent| parent.join(PRIVATE_VAULT_CONTAINER))
}

fn contains(folder: &Path, path: &Path) -> bool {
    let mut parts = path.components();
    folder.components().all(|part| {
        parts.next().is_some_and(|other| {
            if cfg!(any(windows, target_os = "macos")) {
                part.as_os_str().eq_ignore_ascii_case(other.as_os_str())
            } else {
                part == other
            }
        })
    })
}

fn scratch(layout: &Layout) -> Option<PathBuf> {
    let data = layout.vibe_home.parent()?;
    // Only a complete scratch-folder name is an exception: neither its parent nor a similar sibling is one.
    // Resolve cwd too, so dots, links and Windows prefixes have the same identity as the path being checked.
    resolve(&layout.cwd).ok().filter(|cwd| {
        let root = data.join("temporary-folders");
        if !contains(&root, cwd) {
            return false;
        }
        let mut parts = cwd.components().skip(root.components().count());
        let Some(Component::Normal(name)) = parts.next() else {
            return false;
        };
        parts.next().is_none()
            && name.to_str().is_some_and(|name| {
                let name = if cfg!(any(windows, target_os = "macos")) {
                    name.to_ascii_lowercase()
                } else {
                    name.to_owned()
                };
                name.strip_prefix("chat-").is_some_and(|hex| {
                    hex.len() == 16
                        && hex
                            .bytes()
                            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
                })
            })
    })
}

pub(super) fn copy_permit(
    layout: &Layout,
    copy: &Path,
    is_own: bool,
    permits: &Permits<'_>,
) -> Result<(), String> {
    if scratch(layout).is_some() {
        Err("temporary chats do not own a chat copy".to_owned())
    } else {
        permits(copy, is_own)
    }
}

pub(super) fn check(layout: &Layout, real: &Path, access: Access, own: &str) -> Option<String> {
    let data = layout.vibe_home.parent()?;
    // The private vault sits beside the app data (the vault's own place rule): never read, changed or worked in.
    if let Some(vault) = private_vault_container(data)
        && contains(&vault, real)
    {
        return Some(format!(
            "Not run: this is the vault ({}); the agent never reads or changes it. {own} Use your own folder for this.",
            shown::shown(real)
        ));
    }
    if !matches!(access, Access::Read | Access::Context) && contains(real, data) {
        return Some(format!(
            "Not run: this folder contains Vibe Work's own app data ({}); the agent does not read or change it. {own} Use your own folder for this.",
            shown::shown(real)
        ));
    }
    if !contains(data, real) || access == Access::Read && contains(&layout.vibe_home, real) {
        return None;
    }
    let scratch = scratch(layout);
    if scratch
        .as_ref()
        .is_some_and(|scratch| contains(scratch, real))
    {
        return None;
    }
    Some(format!(
        "Not run: Vibe Work's own app data ({}); the agent does not read or change it. {own} Use your own folder for this.",
        shown::shown(real)
    ))
}
