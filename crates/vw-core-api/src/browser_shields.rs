//! Browser ad-filter settings and first-use list downloads. Depends on the app network settings and native filters.
//! Called only by the Browser command; a failed download, compile or save keeps the confirmed mode.
use crate::browser_view::{BrowserRequest, BrowserResult};
use crate::core::Shared;
use crate::error::{CoreError, ErrorCode};
use vw_preview::shields::{MAX_LIST_BYTES, Mode};

fn failed(error: impl std::fmt::Display) -> CoreError {
    CoreError::new(ErrorCode::Internal, format!("Ad blocking: {error}"))
}

fn valid_list(bytes: &[u8]) -> bool {
    bytes.len() <= MAX_LIST_BYTES
        && bytes.starts_with(b"[Adblock")
        && std::str::from_utf8(bytes).is_ok()
}

async fn lists(shared: &Shared) -> Result<String, CoreError> {
    let root = shared
        .config
        .preview
        .as_ref()
        .ok_or_else(|| failed("Browser unavailable"))?
        .config
        .storage_root
        .join("filter-lists");
    vw_platform::ensure_private_dir(&root).map_err(failed)?;
    let client = vw_mistral::network::current()
        .apply(reqwest::Client::builder())
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(45))
        .build()
        .map_err(failed)?;
    let mut lists = String::new();
    for name in ["easylist", "easyprivacy"] {
        let path = root.join(format!("{name}.txt"));
        let cached = if std::fs::metadata(&path).is_ok_and(|m| m.len() <= MAX_LIST_BYTES as u64) {
            std::fs::read(&path).ok().filter(|bytes| valid_list(bytes))
        } else {
            None
        };
        let bytes = if let Some(bytes) = cached {
            bytes
        } else {
            let mut response = client
                .get(format!("https://easylist.to/easylist/{name}.txt"))
                .send()
                .await
                .map_err(|_| failed("list download failed"))?
                .error_for_status()
                .map_err(|_| failed("list download failed"))?;
            let mut bytes = Vec::new();
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|_| failed("list download failed"))?
            {
                if bytes.len() + chunk.len() > MAX_LIST_BYTES {
                    return Err(failed("filter list too large"));
                }
                bytes.extend_from_slice(&chunk);
            }
            if !valid_list(&bytes) {
                return Err(failed("invalid filter list"));
            }
            // Settings requests serialize below, so this temporary belongs to this writer only.
            let temporary = path.with_extension("download");
            if temporary.exists() {
                std::fs::remove_file(&temporary).map_err(failed)?;
            }
            vw_platform::write_private_new(&temporary, &bytes).map_err(failed)?;
            std::fs::rename(&temporary, &path).map_err(failed)?;
            bytes
        };
        lists.push_str(std::str::from_utf8(&bytes).map_err(failed)?);
        lists.push('\n');
    }
    Ok(lists)
}

pub(crate) async fn run(
    shared: &Shared,
    request: &BrowserRequest,
) -> Result<BrowserResult, CoreError> {
    static UPDATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let service = &crate::previews::hub(shared)?.service;
    if let BrowserRequest::SetShields { mode } = request {
        if !service.shields().supported {
            return Err(failed("Unavailable on this system"));
        }
        let _operation = UPDATE.lock().await;
        let lists = if *mode == Mode::Strict {
            Some(lists(shared).await?)
        } else {
            None
        };
        let mode = *mode;
        let rules = tokio::task::spawn_blocking(move || {
            vw_preview::shields::compile(mode, lists.as_deref())
        })
        .await
        .map_err(failed)?
        .map_err(failed)?;
        service.set_shields(mode, rules).await.map_err(failed)?;
        shared.emit(crate::event::CoreEvent::BrowserChanged { session_id: None });
    }
    Ok(BrowserResult::Shields {
        settings: service.shields(),
    })
}
