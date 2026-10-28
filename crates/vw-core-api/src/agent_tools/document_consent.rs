//! Project documents use the chat's access mode and the existing attachment save/read path.
//! Confined file handles keep reads in the chat's folders; Ask binds Allow to the bytes named on its card.
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use cap_std::fs::Dir;
use serde_json::{Value, json};

use crate::core::Shared;
use crate::saved_documents::{self, Digest};
use crate::{CommandResult, DocumentsRequest, DocumentsResult};

struct File {
    path: PathBuf,
    name: String,
    data: Vec<u8>,
    digest: Digest,
    kind: vw_documents::DocKind,
}

fn read_file(folders: &[PathBuf], path: &str) -> Result<File, String> {
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
        .map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
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

pub(super) async fn attach(
    shared: &Arc<Shared>,
    session: &str,
    source: &str,
    call: &str,
    args: &mut Value,
) -> Result<(), String> {
    let path = args["path"]
        .as_str()
        .ok_or("path must name a document")?
        .to_owned();
    let folders = super::session::folders(shared, session)?;
    let cwd = folders[0].clone();
    let file = tokio::task::spawn_blocking(move || read_file(&folders, &path))
        .await
        .map_err(|e| e.to_string())??;
    if let Ok(chat) = shared.saved_documents.chat(session)
        && let Some(saved) = chat.source(&file.path, file.digest)
    {
        args["path"] = json!(cwd.join(saved));
        return Ok(());
    }
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
    if !matches!(
        shared.access.remembered(session).as_deref(),
        Some("auto-approve" | "accept-edits")
    ) {
        crate::ask::tool_calls::approve(shared, session, source, call, detail).await?;
    }
    // Use the same store validation, saving, grant and extraction as an attachment.
    let info = shared
        .documents
        .add_bytes(&file.name, file.data)
        .map_err(|e| e.to_string())?;
    let saved = crate::documents::execute(
        shared,
        DocumentsRequest::Save {
            id: info.id.clone(),
            session_id: session.into(),
        },
    )
    .await;
    shared.documents.remove(&info.id);
    let CommandResult::Documents {
        result: DocumentsResult::Saved { path },
    } = saved.map_err(|e| e.message)?
    else {
        return Err("The document could not be attached.".into());
    };
    let relative = Path::new(&path)
        .strip_prefix(&cwd)
        .map_err(|_| format!("Saved attachment {path} is outside this chat's folder."))?
        .to_path_buf();
    shared
        .saved_documents
        .chat(session)?
        .link_source(file.path, relative, file.digest)?;
    args["path"] = json!(path);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
