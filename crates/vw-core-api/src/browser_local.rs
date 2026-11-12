//! Verifies local PDF/HTML/image navigation through the same confined, excluded-file-aware loader as uploads.
//! Both built-in and personal-browser navigation call this before granting one local document.
use crate::{core::Shared, read_exclusions::ReadExclusions};
use std::sync::Arc;
use url::Url;

pub(crate) async fn prepare(
    shared: &Arc<Shared>,
    owner: &str,
    page: &str,
    text: &str,
) -> Result<Option<Url>, crate::CoreError> {
    if !text.starts_with("file:") {
        return Ok(None);
    }
    let url = checked(shared, owner, text)
        .await
        .map_err(|message| crate::CoreError::new(crate::ErrorCode::Denied, message))?;
    crate::previews::hub(shared)?
        .service
        .allow_local_document(page, &url)
        .map_err(|e| crate::CoreError::new(crate::ErrorCode::Denied, e.to_string()))?;
    Ok(Some(url))
}

pub(crate) async fn checked(shared: &Arc<Shared>, chat: &str, text: &str) -> Result<Url, String> {
    load(shared, chat, text).await.map(|(url, _)| url)
}

pub(crate) async fn load(
    shared: &Arc<Shared>,
    chat: &str,
    text: &str,
) -> Result<(Url, vw_browser::UploadFile), String> {
    let url = Url::parse(text)
        .map_err(|_| "Invalid local file URL; choose a file URL inside the chat folder.")?;
    if url.scheme() != "file" || url.host_str().is_some() || url.query().is_some() {
        return Err("Local files need a file URL without a host or query; choose a file inside the chat folder.".into());
    }
    let path = url
        .to_file_path()
        .map_err(|()| "The local file path is invalid; choose a local file URL.")?;
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if ![
        "pdf", "html", "htm", "png", "jpg", "jpeg", "gif", "webp", "avif", "svg", "ico",
    ]
    .contains(&ext.as_str())
    {
        return Err(
            "This file type cannot be opened as a page; choose PDF, HTML or an image.".into(),
        );
    }
    let folders = crate::agent_tools::session::folders(shared, chat)?;
    let root = folders
        .first()
        .ok_or("Local files need a chat folder; choose the folder containing this file.")?;
    let exclusions = ReadExclusions::for_chat(shared, root)?;
    let path = path
        .to_str()
        .ok_or("The local file path is not valid text; rename the file and retry.")?
        .to_owned();
    let file = tokio::task::spawn_blocking(move || {
        crate::browser_upload::read(&folders, &path, vw_browser::MAX_UPLOAD_BYTES, &exclusions)
    })
    .await
    .map_err(|_| "The local file check did not finish; select the file again.")??;
    Ok((url, file))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn local_navigation_checks_chat_scope_links_size_and_protected_state() {
        let f = crate::feature_test_core::Fixture::new().await;
        f.load("broad", &f.root, &[], false);
        let local = f.root.join("article.html");
        std::fs::write(&local, "<h1>Local</h1>").unwrap();
        let url = Url::from_file_path(&local).unwrap();
        assert!(checked(&f.core.shared, "broad", url.as_str()).await.is_ok());
        let outside = f.root.parent().unwrap().join("other.html");
        assert!(
            checked(
                &f.core.shared,
                "broad",
                Url::from_file_path(outside).unwrap().as_str()
            )
            .await
            .is_err()
        );
        let secret = f.root.join("app-data/state/private.html");
        std::fs::create_dir_all(secret.parent().unwrap()).unwrap();
        std::fs::write(&secret, "private").unwrap();
        assert!(
            checked(
                &f.core.shared,
                "broad",
                Url::from_file_path(secret).unwrap().as_str()
            )
            .await
            .unwrap_err()
            .contains("vault or app state")
        );
        let large = f.root.join("large.pdf");
        std::fs::File::create(&large)
            .unwrap()
            .set_len(vw_browser::MAX_UPLOAD_BYTES as u64 + 1)
            .unwrap();
        assert!(
            checked(
                &f.core.shared,
                "broad",
                Url::from_file_path(large).unwrap().as_str()
            )
            .await
            .is_err()
        );
        #[cfg(unix)]
        {
            let link = f.root.join("link.html");
            std::os::unix::fs::symlink(local, &link).unwrap();
            assert!(
                checked(
                    &f.core.shared,
                    "broad",
                    Url::from_file_path(link).unwrap().as_str()
                )
                .await
                .is_err()
            );
        }
        f.finish().await;
    }
}
