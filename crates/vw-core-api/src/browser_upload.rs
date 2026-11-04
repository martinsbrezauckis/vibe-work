//! Loads the files of a `browser_act upload` from the chat's folders, as the agent named them (relative to the
//! chat's folder, or absolute inside one of its folders). Each file is opened through a directory handle of its
//! folder (cap-std), without following any links or reparse points; at most 16 files and 32 MB together.
//!
//! Depends on: `agent_tools::session::folders`, vw-browser (`Action::Upload`). Called by: the agent's
//! `browser_act` and the Debug API's browser act, before the driver runs the upload.

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use vw_browser::{Action, MAX_UPLOAD_BYTES, UploadFile};

use crate::core::Shared;
use crate::read_exclusions::ReadExclusions;

/// Fills an upload action's files from `owner`'s chat folders; other actions pass unchanged.
pub(crate) async fn load(
    shared: &Arc<Shared>,
    owner: &str,
    action: Action,
) -> Result<Action, String> {
    let Action::Upload {
        reference, paths, ..
    } = action
    else {
        return Ok(action);
    };
    let folders = crate::agent_tools::session::folders(shared, owner)?;
    let exclusions = ReadExclusions::for_chat(shared, &folders[0])?;
    let named = paths.clone();
    let files = tokio::task::spawn_blocking(move || read_all(&folders, &named, &exclusions))
        .await
        .map_err(|e| e.to_string())??;
    Ok(Action::Upload {
        reference,
        paths,
        files,
    })
}

fn read_all(
    folders: &[PathBuf],
    paths: &[String],
    exclusions: &ReadExclusions,
) -> Result<Vec<UploadFile>, String> {
    let mut total = 0usize;
    let mut files = Vec::new();
    for path in paths {
        let file = read(folders, path, MAX_UPLOAD_BYTES - total, exclusions)?;
        total += file.data.len();
        files.push(file);
    }
    Ok(files)
}

/// The folder a path lies in and the path inside it (relative paths: the chat's first folder).
fn locate<'a>(folders: &'a [PathBuf], path: &str) -> Option<(&'a Path, PathBuf)> {
    let named = Path::new(path);
    if named.is_absolute() {
        return folders.iter().find_map(|folder| {
            named
                .strip_prefix(folder)
                .ok()
                .map(|inside| (folder.as_path(), inside.to_path_buf()))
        });
    }
    folders
        .first()
        .map(|folder| (folder.as_path(), named.to_path_buf()))
}

fn read(
    folders: &[PathBuf],
    path: &str,
    room: usize,
    exclusions: &ReadExclusions,
) -> Result<UploadFile, String> {
    let outside = || {
        format!(
            "{path} is outside this chat's folders: upload attaches files from this chat's folders only. Copy the \
             file into the chat's folder first if you need it."
        )
    };
    let (folder, inside) = locate(folders, path).ok_or_else(outside)?;
    if inside
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(outside());
    }
    // Search's no-follow walk opens every component through its parent handle, including the leaf.
    let file = crate::search_walk::open(folder, &inside.to_string_lossy()).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            format!("{path} was not found in this chat's folder")
        } else if error.to_string() == "not a file" {
            format!("{path} is not a file")
        } else {
            format!("{path} cannot be opened without following a link or reparse point, or access was denied. Choose a file with no links in its path that you can read.")
        }
    })?;
    let file = file.into_std();
    exclusions.check_opened(&file, &folder.join(&inside), None)?;
    let meta = file.metadata().map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err(format!("{path} is not a file"));
    }
    let too_large = || {
        format!(
            "the files are larger than {} MB together: attach fewer or smaller files",
            MAX_UPLOAD_BYTES / (1024 * 1024)
        )
    };
    if meta.len() > room as u64 {
        return Err(too_large());
    }
    let mut data = Vec::new();
    file.take(room as u64 + 1)
        .read_to_end(&mut data)
        .map_err(|_| format!("{path} cannot be read"))?;
    if data.len() > room {
        return Err(too_large());
    }
    let name = inside
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|n| !n.is_empty())
        .ok_or_else(|| format!("{path} has no file name"))?
        .to_owned();
    Ok(UploadFile {
        mime: crate::work::mime_of(&name).to_owned(),
        name,
        data,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upload_refuses_an_uncleared_hard_link_in_a_large_protected_tree() {
        let (root, policy, _) = crate::read_exclusions::tests::fixture("upload-bounded");
        crate::read_exclusions::tests::large_protected_tree(&root);
        let original = root.join("ordinary.txt");
        let alias = root.join("alias.txt");
        std::fs::write(&original, "ordinary").unwrap();
        assert!(
            super::read_all(
                std::slice::from_ref(&root),
                &[original.display().to_string()],
                &policy
            )
            .is_ok()
        );
        std::fs::hard_link(&original, &alias).unwrap();
        let error = super::read_all(
            std::slice::from_ref(&root),
            &[alias.display().to_string()],
            &policy,
        )
        .expect_err("unbounded protected walk allowed the upload");
        assert!(
            error.contains("vault or app state") && error.contains("Choose a file"),
            "{error}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn upload_refuses_a_vault_hard_link_in_a_full_access_chat() {
        let f = crate::feature_test_core::Fixture::new().await;
        f.load("chat", &f.root, &[], false);
        f.core.shared.access.observe_runtime(
            "chat",
            &serde_json::json!({"activeAgent":{"name":"auto-approve"}}),
        );
        let private =
            vw_vault::private_location::private_vault_dir(&f.root.join("app-data")).unwrap();
        std::fs::create_dir_all(&private).unwrap();
        let profile = private.join("profile.json");
        std::fs::write(&profile, br#"{"token":"synthetic-profile"}"#).unwrap();
        let alias = f.root.join("notes.json");
        std::fs::hard_link(profile, &alias).unwrap();
        let error = load(
            &f.core.shared,
            "chat",
            Action::Upload {
                reference: "file-field".into(),
                paths: vec![alias.display().to_string()],
                files: vec![],
            },
        )
        .await
        .expect_err("hard-linked vault profile was uploaded");
        assert!(
            error.contains("vault or app state") && error.contains("Choose a file"),
            "{error}"
        );
        f.finish().await;
    }

    #[test]
    fn upload_refuses_every_known_vault_location() {
        let (root, policy, paths) = crate::read_exclusions::tests::fixture("upload");
        for path in paths {
            let error = super::read_all(
                std::slice::from_ref(&root),
                &[path.display().to_string()],
                &policy,
            )
            .expect_err("vault profile was uploaded");
            assert!(
                error.contains("vault or app state") && error.contains("Choose a file"),
                "{error}"
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn upload_refuses_vault_and_app_state_in_a_broad_chat() {
        let f = crate::feature_test_core::Fixture::new().await;
        f.load("broad", &f.root, &[], false);
        let private =
            vw_vault::private_location::private_vault_dir(&f.root.join("app-data")).unwrap();
        for folder in [
            private,
            f.root.join("app-data/vault"),
            f.root.join("app-data/state"),
        ] {
            std::fs::create_dir_all(&folder).unwrap();
            let path = folder.join("profile.json");
            std::fs::write(&path, br#"{"token":"synthetic-profile"}"#).unwrap();
            let result = load(
                &f.core.shared,
                "broad",
                Action::Upload {
                    reference: "file-field".into(),
                    paths: vec![path.display().to_string()],
                    files: vec![],
                },
            )
            .await;
            let error = result.expect_err("protected profile was uploaded");
            assert!(
                error.contains("vault or app state") && error.contains("Choose a file"),
                "{error}"
            );
        }
        f.finish().await;
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn upload_refuses_in_folder_links_in_every_component() {
        let root = folder("inside-links");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("docs/note.txt", root.join("chat/report.txt")).unwrap();
            std::os::unix::fs::symlink("docs", root.join("chat/linked-docs")).unwrap();
        }
        #[cfg(windows)]
        {
            use crate::read_exclusions::tests::test_link_created;
            let test = "browser_upload::tests::upload_refuses_in_folder_links_in_every_component";
            if !test_link_created(
                std::os::windows::fs::symlink_file("docs/note.txt", root.join("chat/report.txt")),
                test,
            ) || !test_link_created(
                std::os::windows::fs::symlink_dir("docs", root.join("chat/linked-docs")),
                test,
            ) {
                std::fs::remove_dir_all(root).unwrap();
                return;
            }
        }
        for path in ["report.txt", "linked-docs/note.txt"] {
            let error = read_all(&[root.join("chat")], &[path.to_owned()])
                .expect_err("an in-folder link was uploaded");
            assert!(
                error.contains("link") && error.contains("Choose a file"),
                "{error}"
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    fn read_all(folders: &[PathBuf], paths: &[String]) -> Result<Vec<UploadFile>, String> {
        let root = &folders[0];
        let exclusions = ReadExclusions::new(
            &root.join("app/vibe"),
            root,
            None,
            &vw_vault::location::ProfileInput::from_env(),
        )
        .unwrap();
        super::read_all(folders, paths, &exclusions)
    }

    fn folder(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("vw-upload-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("chat/docs")).unwrap();
        std::fs::write(root.join("chat/docs/note.txt"), b"hello").unwrap();
        std::fs::write(root.join("secret.txt"), b"outside").unwrap();
        vw_platform::canonical_plain(&root).unwrap()
    }

    #[test]
    fn reads_chat_files_and_refuses_everything_outside() {
        let root = folder("confined");
        let chat = vec![root.join("chat")];
        let note = read_all(&chat, &["docs/note.txt".to_owned()]).unwrap();
        assert_eq!(
            (
                note[0].name.as_str(),
                note[0].data.as_slice(),
                note[0].mime.as_str()
            ),
            ("note.txt", &b"hello"[..], "text/plain")
        );
        assert_eq!(
            read_all(&chat, &["./docs/note.txt".to_owned()]).unwrap()[0].data,
            b"hello"
        );
        let absolute = root.join("chat/docs/note.txt").display().to_string();
        assert_eq!(read_all(&chat, &[absolute]).unwrap()[0].data, b"hello");
        for escape in ["../secret.txt", root.join("secret.txt").to_str().unwrap()] {
            let refused = read_all(&chat, &[escape.to_owned()]).unwrap_err();
            assert!(
                refused.contains("outside this chat's folders"),
                "{escape}: {refused}"
            );
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(root.join("secret.txt"), root.join("chat/link.txt"))
                .unwrap();
            let refused = read_all(&chat, &["link.txt".to_owned()]).unwrap_err();
            assert!(
                refused.contains("link") && refused.contains("Choose a file"),
                "{refused}"
            );
        }
        assert!(
            read_all(&chat, &["docs".to_owned()])
                .unwrap_err()
                .contains("is not a file")
        );
        assert!(
            read_all(&chat, &["missing.txt".to_owned()])
                .unwrap_err()
                .contains("was not found")
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn refuses_files_over_the_total_size() {
        let root = folder("size");
        let big = vec![0u8; MAX_UPLOAD_BYTES / 2 + 1];
        std::fs::write(root.join("chat/a.bin"), &big).unwrap();
        std::fs::write(root.join("chat/b.bin"), &big).unwrap();
        let chat = vec![root.join("chat")];
        assert_eq!(
            read_all(&chat, &["a.bin".to_owned()]).unwrap()[0]
                .data
                .len(),
            big.len()
        );
        let refused = read_all(&chat, &["a.bin".to_owned(), "b.bin".to_owned()]).unwrap_err();
        assert!(refused.contains("larger than 32 MB together"), "{refused}");
        let _ = std::fs::remove_dir_all(&root);
    }
}
