//! Filesystem fixture matching Vibe 2.26.1 package checkout modes (0500 dirs, 0400 files).
//! Depends on std; used only by cleanup tests, including Windows attribute equivalents.

#![allow(clippy::unwrap_used)]

use std::fs;
use std::path::Path;

/// Creates a hardened checkout with a manifest and one skill.
pub fn readonly_package(root: &Path) {
    let skill = root.join("skills/example");
    fs::create_dir_all(&skill).unwrap();
    fs::write(root.join("plugin.json"), b"{}").unwrap();
    fs::write(skill.join("SKILL.md"), b"skill").unwrap();
    for file in [root.join("plugin.json"), skill.join("SKILL.md")] {
        let mut perms = fs::metadata(&file).unwrap().permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            perms.set_mode(0o400);
        }
        #[cfg(windows)]
        perms.set_readonly(true);
        fs::set_permissions(file, perms).unwrap();
    }
    for dir in [skill, root.join("skills"), root.to_path_buf()] {
        let mut perms = fs::metadata(&dir).unwrap().permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            perms.set_mode(0o500);
        }
        #[cfg(windows)]
        perms.set_readonly(true);
        fs::set_permissions(dir, perms).unwrap();
    }
}
