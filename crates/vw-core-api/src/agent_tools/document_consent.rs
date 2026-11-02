//! Project documents use the chat's access mode and read the original without saving a copy.
//! Confined file handles keep reads in the chat's folders; Ask binds Allow to the bytes named on its card.
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use cap_std::fs::Dir;
use serde_json::{Value, json};

use crate::ask::tool_calls::receipts::Execution;
use crate::core::Shared;
use crate::read_exclusions::ReadExclusions;
use crate::saved_documents::{self, Digest};

struct File {
    path: PathBuf,
    name: String,
    data: Vec<u8>,
    digest: Digest,
    kind: vw_documents::DocKind,
}

fn read_file(folders: &[PathBuf], path: &str, exclusions: &ReadExclusions) -> Result<File, String> {
    let first = folders.first().ok_or("This chat has no folders.")?;
    let path = Path::new(path);
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        first.join(path)
    };
    let path = vw_platform::canonical_plain(&joined).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            format!("No file at {}.", joined.display())
        } else {
            format!("Cannot read {}: {error}", joined.display())
        }
    })?;
    let root = folders
        .iter()
        .find(|root| path.starts_with(root))
        .ok_or_else(|| format!("{} is outside this chat's folders.", path.display()))?;
    let relative = path.strip_prefix(root).map_err(|e| e.to_string())?;
    let dir =
        Dir::open_ambient_dir(root, cap_std::ambient_authority()).map_err(|e| e.to_string())?;
    // Windows refuses to open a folder as a file ("Access is denied"): name the real problem first.
    if dir.metadata(relative).is_ok_and(|meta| meta.is_dir()) {
        return Err(format!("{} is not a file.", path.display()));
    }
    // The handle enforces confinement if a parent changes after canonicalization.
    let mut file = dir
        .open(relative)
        .map_err(|e| format!("Cannot read {}: {e}", path.display()))?
        .into_std();
    exclusions.check_opened(&file, &path, None)?;
    let meta = file.metadata().map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err(format!("{} is not a file.", path.display()));
    }
    if meta.len() > vw_documents::MAX_FILE_BYTES {
        return Err("The document is larger than 50 MB.".into());
    }
    let mut data = Vec::new();
    file.by_ref()
        .take(vw_documents::MAX_FILE_BYTES + 1)
        .read_to_end(&mut data)
        .map_err(|e| e.to_string())?;
    if data.len() as u64 > vw_documents::MAX_FILE_BYTES {
        return Err("The document is larger than 50 MB.".into());
    }
    let name = path
        .file_name()
        .ok_or_else(|| format!("{} is not a file.", path.display()))?
        .to_string_lossy()
        .into_owned();
    let kind = vw_documents::detect(&name, &data)
        .map_err(|_| format!("{name} is not a document Vibe Work can read."))?;
    if kind.method() == vw_documents::ReadMethod::File {
        return Err(format!("{name} is not a document Vibe Work can read."));
    }
    Ok(File {
        path,
        name,
        digest: saved_documents::digest(&data),
        data,
        kind,
    })
}

pub(super) fn registered(shared: &Shared, session: &str, args: &Value) -> bool {
    let Some(path) = args["path"].as_str() else {
        return false;
    };
    let Ok(folders) = super::session::folders(shared, session) else {
        return false;
    };
    let cwd = &folders[0];
    let path = Path::new(path);
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    joined.strip_prefix(cwd).is_ok_and(|relative| {
        shared
            .saved_documents
            .chat(session)
            .is_ok_and(|chat| chat.expected(relative).is_ok())
    })
}

pub(super) async fn prepare(
    shared: &Arc<Shared>,
    session: &str,
    source: &str,
    call: &str,
    args: &Value,
) -> Result<Execution, String> {
    let path = args["path"]
        .as_str()
        .ok_or("path must name a document")?
        .to_owned();
    let folders = super::session::folders(shared, session)?;
    let exclusions = ReadExclusions::for_chat(shared, &folders[0])?;
    let file = tokio::task::spawn_blocking(move || read_file(&folders, &path, &exclusions))
        .await
        .map_err(|e| e.to_string())??;
    let allowed = shared
        .saved_documents
        .chat(session)
        .is_ok_and(|chat| chat.authorize(&file.path, file.digest).is_ok());
    let pages = vw_documents::page_count(file.kind, &file.data).map_or_else(String::new, |count| {
        if count == 1 {
            " (1 page)".to_owned()
        } else {
            format!(" ({count} pages)")
        }
    });
    let ocr = file.kind.method() == vw_documents::ReadMethod::Ocr;
    let summary = if ocr {
        format!("Read {}{pages} with Mistral OCR?", file.name)
    } else {
        format!("Read {}{pages} on this computer?", file.name)
    };
    let detail = json!({"kind":"approval", "choices":["approve", "deny"], "requiredPermissions":[],
        "effect":{"kind":"tool", "toolName":"mcp_vw.document_read", "input":args,
            "vwDocumentDigest":file.digest,
            "display":{"summary":summary, "content":if ocr {
                format!("{}\nIts pages go to Mistral OCR.", file.path.display())
            } else { file.path.display().to_string() }}}});
    if !allowed
        && !matches!(
            shared.access.remembered(session).as_deref(),
            Some("auto-approve" | "accept-edits")
        )
    {
        crate::ask::tool_calls::approve(shared, session, source, call, detail).await?;
    }
    let chat =
        shared
            .saved_documents
            .allow_source(shared, session, file.path.clone(), file.digest)?;
    let shared = shared.clone();
    let mut args = args.clone();
    args["path"] = json!(file.path);
    // The receipt holds the checked bytes privately until execution. No path is reopened after consent.
    Ok(Box::pin(async move {
        super::document_tools::read_bytes(
            &shared,
            chat,
            file.path,
            file.name,
            file.data,
            file.digest,
            &args,
        )
        .await
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_refuses_an_uncleared_hard_link_in_a_large_protected_tree() {
        let (root, policy, _) = crate::read_exclusions::tests::fixture("document-bounded");
        crate::read_exclusions::tests::large_protected_tree(&root);
        let original = root.join("ordinary.pdf");
        let alias = root.join("alias.pdf");
        std::fs::write(&original, b"%PDF-1.7\n").unwrap();
        assert!(
            super::read_file(
                std::slice::from_ref(&root),
                original.to_str().unwrap(),
                &policy
            )
            .is_ok()
        );
        std::fs::hard_link(&original, &alias).unwrap();
        let error = super::read_file(
            std::slice::from_ref(&root),
            alias.to_str().unwrap(),
            &policy,
        )
        .err()
        .expect("unbounded protected walk allowed the document");
        assert!(
            error.contains("vault or app state") && error.contains("Choose a file"),
            "{error}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn document_refuses_a_vault_hard_link_in_a_full_access_chat() {
        let f = crate::feature_test_core::Fixture::new().await;
        f.load("chat", &f.root, &[], false);
        f.core
            .shared
            .access
            .observe_runtime("chat", &json!({"activeAgent":{"name":"auto-approve"}}));
        let private =
            vw_vault::private_location::private_vault_dir(&f.root.join("app-data")).unwrap();
        std::fs::create_dir_all(&private).unwrap();
        let profile = private.join("profile.json");
        std::fs::write(&profile, br#"{"token":"synthetic-profile"}"#).unwrap();
        let alias = f.root.join("notes.json");
        std::fs::hard_link(profile, &alias).unwrap();
        let error = prepare(
            &f.core.shared,
            "chat",
            "test",
            "read",
            &json!({"path": alias}),
        )
        .await
        .err()
        .expect("hard-linked vault profile was read");
        assert!(
            error.contains("vault or app state") && error.contains("Choose a file"),
            "{error}"
        );
        f.finish().await;
    }

    #[test]
    fn document_refuses_every_known_vault_location() {
        let (root, policy, paths) = crate::read_exclusions::tests::fixture("document");
        for path in paths {
            let error =
                super::read_file(std::slice::from_ref(&root), path.to_str().unwrap(), &policy)
                    .err()
                    .unwrap();
            assert!(
                error.contains("vault or app state") && error.contains("Choose a file"),
                "{error}"
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn document_refuses_vault_and_app_state_in_a_broad_chat() {
        let f = crate::feature_test_core::Fixture::new().await;
        f.load("broad", &f.root, &[], false);
        f.core
            .shared
            .access
            .observe_runtime("broad", &json!({"activeAgent":{"name":"auto-approve"}}));
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
            let result = prepare(
                &f.core.shared,
                "broad",
                "test",
                "read",
                &json!({"path":path}),
            )
            .await;
            let error = result.err().unwrap();
            assert!(
                error.contains("vault or app state") && error.contains("Choose a file"),
                "{error}"
            );
        }
        f.finish().await;
    }

    fn read_file(folders: &[PathBuf], path: &str) -> Result<File, String> {
        let root = folders.first().cloned().unwrap_or_else(std::env::temp_dir);
        let exclusions = ReadExclusions::new(
            &root.join("app/vibe"),
            &root,
            None,
            &vw_vault::location::ProfileInput::from_env(),
        )
        .unwrap();
        super::read_file(folders, path, &exclusions)
    }

    #[test]
    fn document_refusals_describe_the_file_problem() {
        let root = std::env::temp_dir().join(format!("document-reasons-{}", std::process::id()));
        std::fs::create_dir_all(root.join("project")).unwrap();
        // The refusal names the real path; macOS's temp folder sits behind a link (/var -> /private/var).
        let root = vw_platform::canonical_plain(&root).unwrap();
        let project = root.join("project");
        let folders = vec![project.clone()];
        let missing = project.join("missing.pdf");
        assert_eq!(
            read_file(&folders, missing.to_str().unwrap())
                .err()
                .unwrap(),
            format!("No file at {}.", missing.display())
        );
        let outside = root.join("outside.txt");
        std::fs::write(&outside, "text").unwrap();
        assert_eq!(
            read_file(&folders, outside.to_str().unwrap())
                .err()
                .unwrap(),
            format!("{} is outside this chat's folders.", outside.display())
        );
        std::fs::write(project.join("binary.bin"), [0, 255, 0]).unwrap();
        assert_eq!(
            read_file(&folders, "binary.bin").err().unwrap(),
            "binary.bin is not a document Vibe Work can read."
        );
        assert_eq!(
            read_file(&[], "any.pdf").err().unwrap(),
            "This chat has no folders."
        );
        let directory = project.join("directory");
        std::fs::create_dir_all(&directory).unwrap();
        assert_eq!(
            read_file(&folders, "directory").err().unwrap(),
            format!("{} is not a file.", directory.display())
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let unreadable = project.join("unreadable.txt");
            std::fs::write(&unreadable, "text").unwrap();
            std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o000)).unwrap();
            let error = read_file(&folders, "unreadable.txt").err().unwrap();
            assert!(
                error.starts_with(&format!("Cannot read {}:", unreadable.display())),
                "{error}"
            );
            std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
