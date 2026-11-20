//! Whether a filesystem entry is a link the workspace never follows: a symbolic link, or on Windows any reparse point
//! (junctions and mount points too, which some Windows versions do not report as symbolic links). Read from
//! `symlink_metadata`, so the entry itself is judged, not what it points to.
//!
//! Depends on: std only. Called by: temporary-chat cleanup and the helper refresh (vw-core-api), the vault location
//! check (vw-vault) and the agent folder guard (vw-agent-tools).

/// True when `meta` (from `symlink_metadata`) is a symbolic link or, on Windows, any reparse point.
#[must_use]
pub fn is_link(meta: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt as _;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
    }
    meta.file_type().is_symlink()
}

#[cfg(test)]
mod tests {
    use super::is_link;

    #[test]
    fn a_link_is_one_and_a_plain_file_or_folder_is_not() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("file");
        let folder = root.path().join("folder");
        std::fs::write(&file, b"x").unwrap();
        std::fs::create_dir(&folder).unwrap();
        assert!(!is_link(&std::fs::symlink_metadata(&file).unwrap()));
        assert!(!is_link(&std::fs::symlink_metadata(&folder).unwrap()));
        let link = root.path().join("link");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&folder, &link).unwrap();
        #[cfg(windows)]
        if std::os::windows::fs::symlink_dir(&folder, &link).is_err() {
            // Symbolic links need Developer Mode or the privilege on Windows; a junction is a reparse point too.
            let status = std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(&link)
                .arg(&folder)
                .status()
                .unwrap();
            assert!(status.success());
        }
        assert!(is_link(&std::fs::symlink_metadata(&link).unwrap()));
        assert!(
            !is_link(&std::fs::metadata(&link).unwrap()),
            "the target is judged when followed"
        );
    }
}
