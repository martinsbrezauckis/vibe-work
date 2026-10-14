//! Share a page as one file, resource reading (see `export`): files inside the handbook read with the size
//! budget through a directory capability, CSS with its `url()`s and `@import`s inlined and every remote reference left
//! in it neutralized, `data:` URLs.
//!
//! Depends on: `links` (resolution inside the handbook), `handbook::files` (inside the root), `paths` (MIME
//! types), base64. Called by: `export`.
//! Invariants:
//! - A file is read only through the handbook folder's directory capability (`cap-std`: `openat2` with
//!   `RESOLVE_BENEATH` on Linux, per-component resolution without following a symlink out of the folder elsewhere), so
//!   there is no window between a path check and the read: a symlink swapped in during the export cannot lead outside.
//!   The opened handle must be a regular file; its size is checked on the handle, and at most the budget is read.
//! - Bounded: opened without blocking (`O_NONBLOCK`: a FIFO cannot hang the
//!   export), regular files only, at most `MAX_ASSET` per file (checked on the handle's metadata and enforced by
//!   `take` while reading), at most `MAX_EMBEDDED` read in total, a repeated asset read once and every use of it
//!   counted against `MAX_OUTPUT`, `@import` chains cut at depth 4 and at a loop. What is left out is listed (in the
//!   file and in the answer), never waited for.
//! - CSS keeps no remote reference: `url()`s become `data:` URLs or empty, `@import`s are inlined or dropped, and any
//!   `http:`, `https:` or protocol-relative string left (`image-set("https://...")`) is replaced.

use std::fmt::Write as _;

use base64::Engine as _;

use crate::export::Files;
use crate::html::clip;
use crate::links::{Target, resolve};
use crate::paths::asset_allowed;

/// Deepest `@import` chain followed.
const MAX_IMPORT_DEPTH: usize = 4;

impl Files<'_> {
    /// CSS with `url()` targets inlined and `@import`s replaced by the imported text (relative to `from`).
    pub(crate) fn css(&mut self, css: &str, from: &str, depth: usize) -> String {
        let mut out = String::with_capacity(css.len());
        let lower = css.to_ascii_lowercase();
        let mut i = 0;
        while i < css.len() {
            let next_url = lower[i..].find("url(").map(|p| i + p);
            let next_import = lower[i..].find("@import").map(|p| i + p);
            let at = match (next_url, next_import) {
                (Some(u), Some(m)) => u.min(m),
                (Some(u), None) => u,
                (None, Some(m)) => m,
                (None, None) => break,
            };
            out.push_str(&css[i..at]);
            if Some(at) == next_import {
                let end = css[at..].find(';').map_or(css.len(), |e| at + e + 1);
                let target = import_target(&css[at..end]);
                match target.filter(|_| depth < MAX_IMPORT_DEPTH) {
                    Some(t) => {
                        let looped = match resolve(from, &t) {
                            Target::Local { path, .. } => self.chain.contains(&path),
                            _ => false,
                        };
                        if looped {
                            self.left_out.push(format!("@import loop in {from}"));
                        } else if let Some((path, text)) = self.read_text(from, &t) {
                            self.chain.push(path.clone());
                            let inner = self.css(&text, &path, depth + 1);
                            self.chain.pop();
                            out.push_str(&inner);
                        }
                    }
                    None => self.left_out.push(format!("@import in {from}")),
                }
                i = end;
                continue;
            }
            let start = at + 4;
            let end = css[start..].find(')').map_or(css.len(), |e| start + e);
            let raw = css[start..end].trim().trim_matches(['"', '\'']).trim();
            match self.data_url(from, raw) {
                Some(url) => {
                    let _ = write!(out, "url(\"{url}\")");
                }
                None => out.push_str("url(\"\")"),
            }
            i = (end + 1).min(css.len());
        }
        if i < css.len() {
            out.push_str(&css[i..]);
        }
        if depth == 0 {
            out = neutralize_remote(&out);
        }
        out
    }

    /// The file `href` names (relative to the handbook path `from`), when it is inside the handbook and fits.
    fn read(&mut self, from: &str, href: &str) -> Option<(String, Vec<u8>)> {
        let href = href.trim();
        if href.is_empty()
            || href.starts_with('#')
            || href.to_ascii_lowercase().starts_with("data:")
        {
            return None;
        }
        let Target::Local { path, .. } = resolve(from, href) else {
            self.left_out
                .push(format!("{} (not inside the handbook)", clip(href, 120)));
            return None;
        };
        let limit = self.budget.min(crate::export::MAX_ASSET);
        let bytes = match read_beneath(&self.dir, &path, limit) {
            Ok(bytes) => bytes,
            Err(ReadError::TooLarge) => {
                self.left_out.push(format!(
                    "{path} (larger than {} MB, or the export's 20 MB of files is used up)",
                    limit / (1024 * 1024)
                ));
                return None;
            }
            Err(ReadError::Io(_)) => {
                self.left_out.push(format!(
                    "{path} (missing, or not a file inside the handbook)"
                ));
                return None;
            }
        };
        self.budget -= bytes.len();
        self.inlined += 1;
        Some((path, bytes))
    }

    pub(crate) fn read_text(&mut self, from: &str, href: &str) -> Option<(String, String)> {
        self.read(from, href)
            .map(|(path, bytes)| (path, String::from_utf8_lossy(&bytes).into_owned()))
    }

    /// `href` as a `data:` URL (an image `data:` URL stays as written); `None` when it cannot be inlined.
    pub(crate) fn data_url(&mut self, from: &str, href: &str) -> Option<String> {
        let trimmed = href.trim();
        if trimmed.to_ascii_lowercase().starts_with("data:") {
            return Some(trimmed.to_owned());
        }
        let path = match resolve(from, href) {
            Target::External => {
                self.left_out
                    .push(format!("{} (network resource)", clip(href, 120)));
                return None;
            }
            Target::Local { path, .. } => path,
            Target::SamePage(_) | Target::Outside => {
                self.left_out
                    .push(format!("{} (not inside the handbook)", clip(href, 120)));
                return None;
            }
        };
        // A repeated asset is read once; every use still counts against the output cap.
        let url = if let Some(url) = self.urls.get(&path) {
            url.clone()
        } else {
            let (path, bytes) = self.read(from, href)?;
            let mime = asset_allowed(&path).map_or("application/octet-stream", |m| {
                m.split(';').next().unwrap_or(m)
            });
            let url = format!(
                "data:{mime};base64,{}",
                base64::engine::general_purpose::STANDARD.encode(bytes)
            );
            self.urls.insert(path, url.clone());
            url
        };
        if url.len() > self.output {
            self.left_out.push(format!(
                "{path} (the file is over its {} MB size limit)",
                crate::export::MAX_OUTPUT / (1024 * 1024)
            ));
            return None;
        }
        self.output -= url.len();
        Some(url)
    }
}

/// The target of an `@import` statement (`@import "x.css";`, `@import url(x.css) screen;`).
fn import_target(statement: &str) -> Option<String> {
    let rest = statement.get(7..)?.trim_start();
    let rest = rest
        .strip_prefix("url(")
        .or_else(|| rest.strip_prefix("URL("))
        .unwrap_or(rest);
    let quote = rest.chars().next()?;
    let target: String = if quote == '"' || quote == '\'' {
        rest[1..].split(quote).next()?.to_owned()
    } else {
        rest.split([')', ' ', ';']).next()?.to_owned()
    };
    Some(target).filter(|t| !t.is_empty())
}

/// Why a capability read failed.
pub(crate) enum ReadError {
    /// Larger than the budget left.
    TooLarge,
    /// Missing, outside the folder, not a regular file, unreadable.
    Io(std::io::Error),
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLarge => f.write_str("larger than the 20 MB budget"),
            Self::Io(error) => write!(f, "{error}"),
        }
    }
}

/// Reads the regular file at `path` (`/`-separated, relative to `dir`) through the directory capability, at most
/// `budget` bytes (checked on the opened handle and while reading).
pub(crate) fn read_beneath(
    dir: &cap_std::fs::Dir,
    path: &str,
    budget: usize,
) -> Result<Vec<u8>, ReadError> {
    use std::io::Read as _;
    let mut options = cap_std::fs::OpenOptions::new();
    options.read(true);
    // A FIFO (or another special file) inside the handbook must not block the open: O_NONBLOCK, then the handle must
    // be a regular file. Regular-file reads ignore the flag.
    #[cfg(unix)]
    {
        use cap_std::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = dir.open_with(path, &options).map_err(ReadError::Io)?;
    let meta = file.metadata().map_err(ReadError::Io)?;
    if !meta.is_file() {
        return Err(ReadError::Io(std::io::Error::other("not a regular file")));
    }
    if meta.len() > budget as u64 {
        return Err(ReadError::TooLarge);
    }
    let mut bytes = Vec::new();
    file.take(budget as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(ReadError::Io)?;
    if bytes.len() > budget {
        return Err(ReadError::TooLarge);
    }
    Ok(bytes)
}

/// `css` with every remote reference left after inlining replaced (`http:`, `https:`, `//host` after a quote or a
/// parenthesis): CSS has more ways to name a URL than `url()` (`image-set("...")`, `src()`), and none may reach the
/// network when the file is opened (the CSP blocks them too; this is the second layer).
pub(crate) fn neutralize_remote(css: &str) -> String {
    let mut out = css.to_owned();
    for needle in ["http://", "https://", "\"//", "'//", "(//"] {
        while let Some(at) = out.to_ascii_lowercase().find(needle) {
            let keep = usize::from(needle.starts_with(['"', '\'', '(']));
            out.replace_range(at + keep..at + needle.len(), "about:invalid#");
        }
    }
    out
}
