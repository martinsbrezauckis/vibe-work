//! Packs the passive live DOM and CSSOM into one bounded HTML document. Uses the existing capture fetcher
//! for URLs the document reported loaded; failures remain absolute links and are counted. Called by capture.
use super::media::{Fetch, sniff};
use base64::Engine as _;
use serde::Deserialize;

/// Largest picture or font before encoding.
pub const MAX_ASSET: usize = 5 * 1024 * 1024;
/// Largest finished file, including encoded resources and the capture header.
pub const MAX_TOTAL: usize = 50 * 1024 * 1024;

/// A resource reference made by the sanitized document.
#[derive(Debug, Deserialize)]
pub struct Asset {
    /// Unique placeholder in markup or CSS.
    pub token: String,
    /// Absolute source URL.
    pub url: String,
    /// Seen loaded by this document.
    pub loaded: bool,
    /// Reference lives in raw stylesheet text rather than an HTML attribute.
    #[serde(default)]
    pub css: bool,
}
/// The page script's passive markup and resources.
#[derive(Debug, Deserialize)]
pub struct Document {
    /// Passive document with absolute links and resource placeholders.
    pub html: String,
    /// Pictures and fonts; stylesheets have already become CSS rules.
    pub assets: Vec<Asset>,
}

fn quoted(url: &str) -> String {
    url.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
}

async fn data(asset: &Asset, session: &str, fetch: &dyn Fetch) -> Option<String> {
    if !asset.loaded {
        return None;
    }
    let bytes = if let Some(data) = asset.url.strip_prefix("data:") {
        let (header, body) = data.split_once(',')?;
        if !header.ends_with(";base64") || body.len() > MAX_ASSET.div_ceil(3) * 4 {
            return None;
        }
        base64::engine::general_purpose::STANDARD
            .decode(body)
            .ok()?
    } else {
        if !url::Url::parse(&asset.url).is_ok_and(|u| matches!(u.scheme(), "http" | "https")) {
            return None;
        }
        tokio::time::timeout(
            super::media::FETCH_TIMEOUT,
            fetch.fetch(session, &asset.url, MAX_ASSET as u64),
        )
        .await
        .ok()?
        .ok()?
        .bytes
    };
    if bytes.len() > MAX_ASSET {
        return None;
    }
    let (_, mime) = sniff(&bytes)?;
    let safe;
    let bytes = if mime == "image/svg+xml" {
        safe = super::svg::sanitize(std::str::from_utf8(&bytes).ok()?)?;
        safe.as_bytes()
    } else {
        bytes.as_slice()
    };
    Some(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

/// Replaces resource placeholders, keeping the complete document under `cap` bytes.
///
/// # Errors
/// The passive document itself is too large, or the resource list is invalid.
pub async fn pack(
    document: Document,
    session: &str,
    fetch: &dyn Fetch,
    cap: usize,
) -> Result<(String, usize), String> {
    let mut html = document.html;
    if html.len() > cap || document.assets.len() > 10_000 {
        return Err("the page exceeds the 50 MB one-file limit".into());
    }
    let mut kept = 0;
    let deadline = tokio::time::Instant::now() + super::media::STEP_LIMIT;
    for asset in document.assets {
        let count = html.matches(&asset.token).count();
        if asset.token.is_empty() || count == 0 {
            continue;
        }
        let link = if asset.css {
            asset.url.clone()
        } else {
            quoted(&asset.url)
        };
        let room = |value: &str| {
            html.len()
                .saturating_sub(asset.token.len() * count)
                .saturating_add(value.len().saturating_mul(count))
                <= cap
        };
        let packed = tokio::time::timeout_at(deadline, data(&asset, session, fetch))
            .await
            .ok()
            .flatten();
        let value = match packed {
            Some(ref uri) if room(uri) => uri,
            _ => {
                kept += 1;
                &link
            }
        };
        if !room(value) {
            return Err("the page exceeds the 50 MB one-file limit".into());
        }
        html = html.replace(&asset.token, value);
    }
    Ok((html, kept))
}
