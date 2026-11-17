//! The files the attach dialog returned, read for the composer: name, size and bytes, never a path.
//!
//! Depends on: base64. Called by: `commands.rs` (`pick_attachments`).
//! Invariants:
//! - The webview gets the file's name and content only; the path stays in Rust.
//! - A file over [`MAX_PICKED_BYTES`] is reported by name and size without its bytes (the composer refuses it by
//!   size before it would read any), so a huge pick never crosses the bridge.
//! - A file that cannot be read is reported with no bytes; its size is 0 when metadata is unavailable.
//!   The composer reports the read failure by name through the shared byte router.

use std::path::PathBuf;

use base64::Engine as _;
use serde::Serialize;

/// The largest file whose bytes are returned: the documents limit the composer's router and the core apply.
pub(crate) const MAX_PICKED_BYTES: u64 = 50 * 1024 * 1024;

/// One picked file as the composer receives it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PickedFile {
    /// The file's name without its folder.
    pub name: String,
    /// Its size in bytes.
    pub size: u64,
    /// Its content in base64; `None` when it is over the limit or unreadable.
    pub data: Option<String>,
}

/// Reads the files `paths` names, in order. Blocking: call it from a blocking task.
pub(crate) fn read_picked(paths: Vec<PathBuf>) -> Vec<PickedFile> {
    paths
        .into_iter()
        .map(|path| {
            let name = path
                .file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
            let size = std::fs::metadata(&path).map_or(0, |meta| meta.len());
            let data = (size <= MAX_PICKED_BYTES)
                .then(|| std::fs::read(&path).ok())
                .flatten()
                .map(|bytes| base64::engine::general_purpose::STANDARD.encode(bytes));
            PickedFile { name, size, data }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    //! Picked files retain their order and names while limiting bytes sent to the composer.
    use super::{MAX_PICKED_BYTES, read_picked};

    #[test]
    fn a_picked_file_arrives_as_name_size_and_base64_without_its_folder() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.txt");
        std::fs::write(&path, b"hello").unwrap();
        let files = read_picked(vec![path]);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].name, "notes.txt");
        assert_eq!(files[0].size, 5);
        assert_eq!(files[0].data.as_deref(), Some("aGVsbG8="));
    }

    #[test]
    fn an_unreadable_file_keeps_its_name_and_carries_no_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let files = read_picked(vec![dir.path().join("gone.pdf")]);
        assert_eq!(files[0].name, "gone.pdf");
        assert_eq!(files[0].size, 0);
        assert_eq!(files[0].data, None);
    }

    #[test]
    fn the_limit_matches_the_documents_limit() {
        assert_eq!(MAX_PICKED_BYTES, 50 * 1024 * 1024);
    }

    #[test]
    fn cancelling_returns_no_files() {
        assert_eq!(read_picked(Vec::new()), Vec::new());
    }

    #[test]
    fn an_empty_file_retains_its_name_and_reads_as_empty_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.txt");
        std::fs::write(&path, b"").unwrap();
        let files = read_picked(vec![path]);
        assert_eq!(files[0].name, "empty.txt");
        assert_eq!(files[0].size, 0);
        assert_eq!(files[0].data.as_deref(), Some(""));
    }

    #[test]
    fn an_oversized_file_reports_its_size_without_reading_its_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("large.pdf");
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(MAX_PICKED_BYTES + 1).unwrap();
        let files = read_picked(vec![path]);
        assert_eq!(files[0].size, MAX_PICKED_BYTES + 1);
        assert_eq!(files[0].data, None);
    }

    #[test]
    fn mixed_files_keep_order_and_never_serialize_their_parent_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes-\u{00e9}.txt");
        std::fs::write(&path, b"hello").unwrap();
        let files = read_picked(vec![path, dir.path().join("missing.pdf")]);
        assert_eq!(files[0].name, "notes-\u{00e9}.txt");
        assert_eq!(files[1].name, "missing.pdf");
        let json = serde_json::to_value(&files).unwrap();
        assert_eq!(json[0]["data"], "aGVsbG8=");
        assert_eq!(json[0].as_object().unwrap().len(), 3);
        assert!(json[0].get("path").is_none());
    }
}
