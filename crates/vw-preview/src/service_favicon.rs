//! Bounded Browser tab icons from the admitted native page. Depends on native evaluation and PNG validation;
//! called by the core's tab-list read. The main UI receives only normalized pixels, never a remote image URL.

use std::time::Duration;

use base64::Engine as _;
use serde::Deserialize;

use super::PreviewService;

const SOURCE: &str = include_str!("js/favicon.js");

#[derive(Deserialize)]
struct IconAnswer {
    url: String,
    done: bool,
    data: Option<String>,
}

fn png_icon(data: &str) -> Option<String> {
    let encoded = data.strip_prefix("data:image/png;base64,")?;
    if encoded.len() > 16_384 {
        return None;
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .ok()?;
    let mut decoder = png::Decoder::new(std::io::Cursor::new(&bytes));
    decoder.set_limits(png::Limits { bytes: 131_072 });
    let reader = decoder.read_info().ok()?;
    let info = reader.info();
    if info.width == 0 || info.height == 0 || info.width > 64 || info.height > 64 {
        return None;
    }
    let shot = crate::shot::shape(&bytes).ok()?;
    Some(format!("data:image/png;base64,{}", shot.base64()))
}

impl PreviewService {
    /// Refreshes an icon once per loaded native document. A missing or inaccessible icon uses the UI fallback.
    /// Icon loads use the native page's existing network policy; no host fetch, cookies or IPC are exposed.
    pub async fn refresh_favicon(&self, page_key: &str) {
        let initial = {
            let state = self.inner.lock();
            let Some(entry) = state.entries.get(page_key) else {
                return;
            };
            let loads = *entry.loads.borrow();
            if entry.loading
                || entry
                    .favicon
                    .as_ref()
                    .is_some_and(|(epoch, _)| *epoch == loads)
            {
                return;
            }
            let (Some(mount), Some(url)) = (&entry.mount, &entry.url) else {
                return;
            };
            (mount.clone(), loads, url.to_string())
        };
        let (mount, loads, url) = initial;
        let id = self.mount_id(page_key, &mount);
        let script = SOURCE.replace(
            "__VW_FAVICON_NAME__",
            &serde_json::json!(mount.page).to_string(),
        );
        let mut icon = None;
        for _ in 0..6 {
            let Ok(Ok(raw)) = tokio::time::timeout(
                Duration::from_millis(300),
                self.evaluate_on_mount(&id, script.clone()),
            )
            .await
            else {
                break;
            };
            if raw.len() > 18_000 {
                break;
            }
            let Ok(answer) = serde_json::from_str::<IconAnswer>(&raw) else {
                break;
            };
            if answer.url != url
                || self
                    .reading_allowed(page_key, &mount.page, &answer.url)
                    .is_err()
            {
                break;
            }
            if answer.done {
                icon = answer.data.as_deref().and_then(png_icon);
                break;
            }
            tokio::time::sleep(Duration::from_millis(80)).await;
        }
        let mut state = self.inner.lock();
        if let Some(entry) = state.entries.get_mut(page_key)
            && !entry.loading
            && *entry.loads.borrow() == loads
            && entry.url.as_ref().map(url::Url::as_str) == Some(url.as_str())
            && entry
                .mount
                .as_ref()
                .is_some_and(|current| current.label == mount.label)
        {
            entry.favicon = Some((loads, icon));
        }
    }
}
