//! Stable download receipts and URL transfers scoped to the chat owning a browser page.
//! Uses the existing staging/delivery hooks; the native engine supplies its profile cookies internally.
//! Depends on: preview staging/delivery hooks and engine transfers.
//! Called by: browser download actions.
use crate::{DownloadState, EngineError, PreviewError, PreviewHooks, PreviewService};
use serde_json::{Value, json};
use std::path::Path;
use url::Url;

impl PreviewService {
    /// List only downloads attributed to this page's chat, with opaque ids and Chrome-compatible state words.
    #[must_use]
    pub fn chat_downloads(&self, page: &str) -> Vec<Value> {
        let Some(owner) = self.owner_session_id(page) else {
            return Vec::new();
        };
        let mut pages = self.web_tabs(&owner);
        pages.push(owner);
        let state = self.inner.lock();
        let mut records: Vec<_> = pages
            .iter()
            .filter_map(|key| state.entries.get(key))
            .flat_map(|entry| entry.downloads.iter())
            .collect();
        records.sort_by_key(|r| r.at);
        records.iter().rev().take(crate::downloads::MAX_LISTED).map(|r| {
            let path=Path::new(&r.view.path);
            let bytes=std::fs::metadata(if r.view.path.is_empty(){&r.staging}else{path}).map_or(0,|m|m.len());
            let (status,finished)=match r.view.state {DownloadState::Running=>("in_progress",false),DownloadState::Done=>("complete",true),DownloadState::Failed=>("interrupted",true)};
            json!({"id":r.staging.file_stem().and_then(|s|s.to_str()),"state":status,"finished":finished,"name":if r.view.path.is_empty(){r.name.as_str()}else{path.file_name().and_then(|s|s.to_str()).unwrap_or(&r.name)},"filename":(!r.view.path.is_empty()).then_some(&r.view.path),"folder":(!r.view.path.is_empty()).then(||r.dir.display().to_string()),"size":finished.then_some(bytes),"bytesReceived":bytes})
        }).collect()
    }

    /// Start an HTTP(S) download into private staging; successful completion delivers it into the chat folder.
    ///
    /// # Errors
    /// Missing live page/folder, invalid URL or staging failure.
    pub async fn download_url(&self, page: &str, url: Url) -> Result<String, PreviewError> {
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(EngineError::Platform(
                "Download needs an HTTP(S) URL without credentials; use a vault login to sign in."
                    .into(),
            )
            .into());
        }
        let _activity = self.session_lock(page).await;
        let id = self.page_mount(page).ok_or(EngineError::NotMounted)?;
        let suggested = url
            .path_segments()
            .and_then(|mut p| p.next_back())
            .filter(|n| !n.is_empty())
            .unwrap_or("download");
        let hooks = crate::hooks::Hooks(self.inner.clone());
        let staged = hooks
            .download_destination(&id.label, &url, Path::new(suggested))
            .ok_or_else(|| {
                EngineError::Platform("This chat has no writable downloads folder.".into())
            })?;
        let download_id = staged
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| EngineError::Platform("Download id unavailable.".into()))?
            .to_owned();
        let engine = self.engine.clone();
        let max = self.inner.max_download_bytes;
        tokio::spawn(async move {
            let ok = engine
                .download_url(&id, url.clone(), staged.clone(), max)
                .await
                .is_ok();
            hooks.download_finished(&id.label, &url, Some(&staged), ok);
        });
        Ok(download_id)
    }
}
