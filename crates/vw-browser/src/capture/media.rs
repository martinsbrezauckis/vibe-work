//! The media capture: logos, icons, images and web fonts the page already loaded, saved under
//! `media/` with a manifest (`media.json`: source URL, kind, type, size, status). The page script (`js/media.js`)
//! only lists; the host fetches through [`Fetch`] (the core: no cookies, the navigation policy per URL, DNS answers
//! checked and pinned, no redirect to anywhere the policy refuses). Manifest shape after ShellX Canvas
//! (`media.json`), which never fetched remote media; this does, within caps.
//!
//! Depends on: serde, `serde_json`, futures-util, tokio, `files`. Called by: `capture`, tests.
//! Invariants:
//! - Only `http`/`https` URLs the page reported as loaded are fetched; at most [`MAX_FILES`] files, [`MAX_FILE`]
//!   bytes each, [`FETCH_TIMEOUT`] each, [`PARALLEL`] at a time, the whole step within [`STEP_LIMIT`].
//! - A file's type comes from its bytes ([`sniff`]); anything that is not an image or a font is listed, not saved.
//!   An SVG is saved only as `svg::sanitize`'s allowlisted re-serialisation (never the page's own markup); one that
//!   does not parse as a plain SVG drawing is listed, not saved.
//! - File names are ours (`NN-<ascii stem>.<ext>`).

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use futures_util::StreamExt as _;
use serde::Deserialize;
use serde_json::{Value, json};

use super::files::{Folder, file_name, slug};

/// Most files saved.
pub const MAX_FILES: usize = 40;
/// Largest file.
pub const MAX_FILE: u64 = 5 * 1024 * 1024;
/// Most media bytes per capture.
pub const MAX_MEDIA_TOTAL: u64 = 25 * 1024 * 1024;
/// One fetch's limit.
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(8);
/// Fetches at a time.
pub const PARALLEL: usize = 4;
/// The whole media step's limit.
pub const STEP_LIMIT: Duration = Duration::from_secs(25);

/// A fetched file.
#[derive(Clone, Debug, Default)]
pub struct Fetched {
    /// The body (at most the asked cap).
    pub bytes: Vec<u8>,
    /// The `Content-Type` the server sent (informational; the type comes from the bytes).
    pub content_type: Option<String>,
}

/// A boxed fetch future.
pub type FetchFuture<'a> = Pin<Box<dyn Future<Output = Result<Fetched, String>> + Send + 'a>>;

/// Fetches one URL for a session's capture (the core: policy, pinned DNS, no cookies, caps).
pub trait Fetch: Send + Sync {
    /// Fetches `url` for `session_id`, at most `max_bytes` (more is an error).
    fn fetch<'a>(&'a self, session_id: &'a str, url: &'a str, max_bytes: u64) -> FetchFuture<'a>;
}

/// One item the page listed.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Item {
    /// `logo`, `icon`, `image`, `background`, `font`.
    pub kind: String,
    /// Absolute URL (absent for an inline SVG).
    pub url: Option<String>,
    /// Inline SVG markup (logos drawn in the page).
    pub svg: Option<String>,
    /// Alternative text / label (page words).
    pub alt: Option<String>,
    /// Natural size.
    pub width: Option<u64>,
    /// Natural size.
    pub height: Option<u64>,
    /// Font family (page words).
    pub family: Option<String>,
    /// Font weight.
    pub weight: Option<String>,
    /// The page fetched it (resource timing, or a decoded image).
    pub loaded: bool,
}

/// What `js/media.js` answers.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Listing {
    /// Address.
    pub url: String,
    /// Title.
    pub title: String,
    /// Loaded font faces.
    pub faces: Vec<Value>,
    /// The items.
    pub items: Vec<Item>,
}

/// The file type of `bytes`: (extension, MIME type), `None` for anything that is not an image or a font.
#[must_use]
pub fn sniff(bytes: &[u8]) -> Option<(&'static str, &'static str)> {
    let starts = |p: &[u8]| bytes.starts_with(p);
    if starts(b"\x89PNG\r\n\x1a\n") {
        return Some(("png", "image/png"));
    }
    if starts(&[0xff, 0xd8, 0xff]) {
        return Some(("jpg", "image/jpeg"));
    }
    if starts(b"GIF87a") || starts(b"GIF89a") {
        return Some(("gif", "image/gif"));
    }
    if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some(("webp", "image/webp"));
    }
    if bytes.len() >= 12
        && &bytes[4..8] == b"ftyp"
        && (&bytes[8..12] == b"avif" || &bytes[8..12] == b"avis")
    {
        return Some(("avif", "image/avif"));
    }
    if starts(&[0, 0, 1, 0]) {
        return Some(("ico", "image/x-icon"));
    }
    if starts(b"wOFF") {
        return Some(("woff", "font/woff"));
    }
    if starts(b"wOF2") {
        return Some(("woff2", "font/woff2"));
    }
    if starts(&[0, 1, 0, 0]) || starts(b"true") {
        return Some(("ttf", "font/ttf"));
    }
    if starts(b"OTTO") {
        return Some(("otf", "font/otf"));
    }
    if svg_root(bytes) {
        return Some(("svg", "image/svg+xml"));
    }
    None
}

/// True when the document's first element is `<svg` (after an optional byte-order mark, white space, an XML
/// declaration and comments); HTML that merely contains an `<svg>` is not an SVG file.
fn svg_root(bytes: &[u8]) -> bool {
    let text = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]).to_ascii_lowercase();
    let mut rest = text.trim_start_matches('\u{feff}').trim_start();
    loop {
        if let Some(after) = rest.strip_prefix("<?xml ") {
            match after.find("?>") {
                Some(end) => rest = after[end + 2..].trim_start(),
                None => return false,
            }
        } else if let Some(after) = rest.strip_prefix("<!--") {
            match after.find("-->") {
                Some(end) => rest = after[end + 3..].trim_start(),
                None => return false,
            }
        } else {
            return rest.starts_with("<svg")
                && rest[4..]
                    .starts_with(|c: char| c.is_ascii_whitespace() || c == '>' || c == '/');
        }
    }
}

/// A stem for the file name: the URL's last path part without its extension, else the kind.
fn stem_of(item: &Item) -> String {
    let from_url = item.url.as_deref().and_then(|u| {
        let path = u.split(['?', '#']).next()?;
        let last = path.rsplit('/').find(|p| !p.is_empty())?;
        Some(last.rsplit_once('.').map_or(last, |(s, _)| s).to_owned())
    });
    let stem = from_url.unwrap_or_default();
    if slug(&stem) == "page" {
        item.kind.clone()
    } else {
        format!("{}-{stem}", item.kind)
    }
}

/// Which items to fetch, in order: logos, loaded icons, loaded fonts, then images by size; at most [`MAX_FILES`].
fn choose(items: &[Item]) -> Vec<usize> {
    let mut order: Vec<usize> = Vec::new();
    let rank = |k: &str| match k {
        "logo" => 0,
        "icon" => 1,
        "font" => 2,
        "image" => 3,
        _ => 4,
    };
    let mut indices: Vec<usize> = (0..items.len()).collect();
    indices.sort_by_key(|i| rank(&items[*i].kind));
    for i in indices {
        if order.len() >= MAX_FILES {
            break;
        }
        if items[i].svg.is_some() || items[i].loaded {
            order.push(i);
        }
    }
    order
}

fn entry(item: &Item) -> Value {
    let mut v = json!({"kind": item.kind});
    if let Some(u) = &item.url {
        v["url"] = json!(u);
    }
    for (key, value) in [
        ("alt", &item.alt),
        ("family", &item.family),
        ("weight", &item.weight),
    ] {
        if let Some(text) = value.as_deref().filter(|t| !t.is_empty()) {
            v[key] = json!(text.chars().take(120).collect::<String>());
        }
    }
    if let (Some(w), Some(h)) = (item.width, item.height) {
        v["naturalSize"] = json!(format!("{w}x{h}"));
    }
    v
}

/// Saves the chosen items into `folder/media/` and returns the manifest's entries (every listed item, with its
/// status: `saved`, `notLoaded`, `skipped` (over the count cap), `notMedia`, `unsafeSvg`, `tooLarge`, `refused`,
/// `failed`).
///
/// # Errors
/// A plain reason when the media folder cannot be made.
pub async fn save(
    folder: &mut Folder,
    listing: &Listing,
    session_id: &str,
    fetch: &dyn Fetch,
) -> Result<Vec<Value>, String> {
    let chosen = choose(&listing.items);
    let mut entries: Vec<Value> = listing.items.iter().map(entry).collect();
    for (i, item) in listing.items.iter().enumerate() {
        if !chosen.contains(&i) {
            entries[i]["status"] = json!(if item.loaded || item.svg.is_some() {
                "skipped"
            } else {
                "notLoaded"
            });
        }
    }
    folder.subfolder("media")?;
    let mut bodies = bodies(listing, &chosen, session_id, fetch).await;
    bodies.sort_by_key(|(i, _)| *i);
    let mut media_total = 0u64;
    for (n, (i, body)) in bodies.into_iter().enumerate() {
        let item = &listing.items[i];
        let bytes = match body {
            Ok(b) => b,
            Err(reason) => {
                let refused = reason.starts_with("refused");
                entries[i]["status"] = json!(if refused { "refused" } else { "failed" });
                entries[i]["reason"] = json!(reason.chars().take(200).collect::<String>());
                continue;
            }
        };
        let Some((ext, mime)) = sniff(&bytes) else {
            entries[i]["status"] = json!("notMedia");
            continue;
        };
        // An SVG is saved only as our allowlisted re-serialisation, never as the page wrote it.
        let bytes = if ext == "svg" {
            let Some(clean) = std::str::from_utf8(&bytes)
                .ok()
                .and_then(super::svg::sanitize)
            else {
                entries[i]["status"] = json!("unsafeSvg");
                entries[i]["reason"] = json!(
                    "not a plain SVG drawing (a DTD, a processing instruction or broken markup): not saved"
                );
                continue;
            };
            clean.into_bytes()
        } else {
            bytes
        };
        let len = bytes.len() as u64;
        if len > MAX_FILE || media_total + len > MAX_MEDIA_TOTAL {
            entries[i]["status"] = json!("tooLarge");
            continue;
        }
        let name = format!("media/{}", file_name(n + 1, &stem_of(item), ext));
        match folder.write(&name, &bytes, MAX_FILE) {
            Ok(w) => {
                media_total += w.bytes;
                entries[i]["status"] = json!("saved");
                entries[i]["file"] = json!(w.name);
                entries[i]["bytes"] = json!(w.bytes);
                entries[i]["type"] = json!(mime);
            }
            Err(reason) => {
                entries[i]["status"] = json!("failed");
                entries[i]["reason"] = json!(reason);
            }
        }
    }
    Ok(entries)
}

/// The bodies of the chosen items, in no particular order: fetched in parallel (bounded) for URLs, the markup for
/// inline SVGs, a time-limit failure for what the step did not reach.
async fn bodies(
    listing: &Listing,
    chosen: &[usize],
    session_id: &str,
    fetch: &dyn Fetch,
) -> Vec<(usize, Result<Vec<u8>, String>)> {
    // Fetch in parallel (bounded), write in order.
    let urls: Vec<(usize, String)> = chosen
        .iter()
        .filter_map(|i| listing.items[*i].url.clone().map(|u| (*i, u)))
        .collect();
    // Each fetch gets at most FETCH_TIMEOUT and never runs past the step's deadline, so finished fetches are kept
    // when the step runs out of time.
    let deadline = tokio::time::Instant::now() + STEP_LIMIT;
    let mut fetched = futures_util::stream::iter(urls.into_iter().map(|(i, url)| async move {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            return (
                i,
                Err("not fetched within the media step's time limit".to_owned()),
            );
        }
        let result = tokio::time::timeout(
            left.min(FETCH_TIMEOUT),
            fetch.fetch(session_id, &url, MAX_FILE),
        )
        .await
        .unwrap_or_else(|_| Err("timed out".to_owned()));
        (i, result)
    }))
    .buffer_unordered(PARALLEL)
    .collect::<Vec<_>>()
    .await;
    fetched.sort_by_key(|(i, _)| *i);
    let mut bodies: Vec<(usize, Result<Vec<u8>, String>)> = fetched
        .into_iter()
        .map(|(i, r)| (i, r.map(|f| f.bytes)))
        .collect();
    for i in chosen {
        if let Some(svg) = &listing.items[*i].svg {
            bodies.push((*i, Ok(svg.clone().into_bytes())));
        } else if !bodies.iter().any(|(j, _)| j == i) {
            bodies.push((
                *i,
                Err("not fetched within the media step's time limit".to_owned()),
            ));
        }
    }
    bodies
}
