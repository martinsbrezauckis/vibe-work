//! Reserves app data outside the readable Vibe home, except for the current chat's scratch folder.
//! Uses the folder guard's kernel-order path resolution; called alongside its existing control-file and copy checks.
use super::{Access, Layout, Permits, resolve, shown};
use std::path::{Component, Path, PathBuf};

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
        Err("temporary chats do not own a session copy".to_owned())
    } else {
        permits(copy, is_own)
    }
}

pub(super) fn check(layout: &Layout, real: &Path, access: Access, own: &str) -> Option<String> {
    let data = layout.vibe_home.parent()?;
    if !matches!(access, Access::Read | Access::Context) && contains(real, data) {
        return Some(format!(
            "Not run: this folder contains Vibe Work's own app data ({}); the agent does not read or change it. {own} Work only in your own folder.",
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
        "Not run: Vibe Work's own app data ({}); the agent does not read or change it. {own} Work only in your own folder.",
        shown::shown(real)
    ))
}
