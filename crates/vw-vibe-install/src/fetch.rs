//! The uv download and unpack: one HTTPS GET of Astral's release archive with a size limit and a
//! timeout, checked against the pinned SHA-256 **before** anything is unpacked; then only the `uv` executable
//! (`uv.exe` on Windows) is taken out of the archive.
//!
//! Network: the shared client of vw-mistral (OS trust store, the system proxy; `HTTPS_PROXY` & co. are honoured).
//! That client never follows redirects (it also carries the key elsewhere); GitHub release assets redirect to their
//! CDN, so this module follows them itself: at most [`MAX_REDIRECTS`] hops, each to `https` (plain `http` only from
//! a loopback address to a loopback address: the tests' local server). No secret is sent; the pinned SHA-256 decides
//! whether the bytes are used.
//! A connection problem is reported as [`FetchError::Network`] with the host, so the setup screen can say
//! "no connection to github.com" instead of a library message.
//!
//! Depends on: vw-mistral (`https_client`), reqwest, sha2, flate2/tar/zip. Called by: [`crate::manager`].
//! Invariants: a file with the wrong checksum is deleted and never unpacked or run; the archive must hold
//! exactly one `uv` executable at `uv-<target>/uv` (tar) or `uv.exe` (zip); nothing else is extracted, and no
//! archive path is ever joined onto the destination (no path traversal is possible).

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::pin::UvAsset;

/// Redirect hops followed at most.
pub const MAX_REDIRECTS: usize = 5;

/// Largest archive accepted (uv's are ~15-18 MB).
pub const MAX_ARCHIVE_BYTES: u64 = 100 * 1024 * 1024;

/// Why the download or unpack failed.
#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    /// No connection to the host (offline, DNS, refused, proxy).
    #[error("no connection to {host}: {detail}")]
    Network {
        /// The host that could not be reached.
        host: String,
        /// The client's short reason.
        detail: String,
    },
    /// The server answered with an error status.
    #[error("{host} answered HTTP {status}")]
    Status {
        /// The host.
        host: String,
        /// The HTTP status.
        status: u16,
    },
    /// Larger than [`MAX_ARCHIVE_BYTES`] or the pinned size.
    #[error("the download is larger than expected ({0} bytes)")]
    TooLarge(u64),
    /// The checksum differs from the pin.
    #[error("the download's SHA-256 is {found}, the pinned one is {expected}: deleted, not used")]
    Checksum {
        /// What arrived.
        found: String,
        /// What the pin says.
        expected: String,
    },
    /// The whole download took longer than its limit.
    #[error("the download did not finish within {0:?}")]
    Timeout(Duration),
    /// The archive does not hold exactly one uv executable.
    #[error("the uv archive is not as expected: {0}")]
    Archive(String),
    /// Writing the file failed.
    #[error("cannot write {path}: {source}")]
    Disk {
        /// The file.
        path: PathBuf,
        /// The OS error.
        source: std::io::Error,
    },
}

fn host_of(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .unwrap_or_else(|| url.to_owned())
}

fn network(url: &str, error: &reqwest::Error) -> FetchError {
    // The innermost cause is the useful one (DNS, refused, TLS); never the URL's query (there is none here).
    let mut detail = error.to_string();
    let mut source = std::error::Error::source(error);
    while let Some(inner) = source {
        detail = inner.to_string();
        source = inner.source();
    }
    FetchError::Network {
        host: host_of(url),
        detail,
    }
}

/// Downloads `asset` to `dest`, checking size and SHA-256. On any failure `dest` does not exist afterwards.
/// `progress` is called with the bytes received so far.
///
/// # Errors
/// [`FetchError`].
pub async fn download(
    asset: &UvAsset,
    dest: &Path,
    limit: Duration,
    mut progress: impl FnMut(u64),
) -> Result<(), FetchError> {
    let result = tokio::time::timeout(limit, fetch_to(asset, dest, &mut progress)).await;
    let outcome = match result {
        Ok(inner) => inner,
        Err(_) => Err(FetchError::Timeout(limit)),
    };
    if outcome.is_err() {
        let _ = tokio::fs::remove_file(dest).await;
    }
    outcome
}

async fn fetch_to(
    asset: &UvAsset,
    dest: &Path,
    progress: &mut impl FnMut(u64),
) -> Result<(), FetchError> {
    let disk = |source| FetchError::Disk {
        path: dest.to_path_buf(),
        source,
    };
    let client = vw_mistral::https_client().map_err(|e| FetchError::Network {
        host: host_of(&asset.url),
        detail: e.to_string(),
    })?;
    let response = follow(&client, &asset.url).await?;
    if !response.status().is_success() {
        return Err(FetchError::Status {
            host: host_of(response.url().as_str()),
            status: response.status().as_u16(),
        });
    }
    let cap = asset.bytes.min(MAX_ARCHIVE_BYTES);
    if let Some(length) = response.content_length()
        && length > cap
    {
        return Err(FetchError::TooLarge(length));
    }
    let mut file = tokio::fs::File::create(dest).await.map_err(disk)?;
    let mut hasher = Sha256::new();
    let mut received = 0_u64;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| network(&asset.url, &e))?;
        received += chunk.len() as u64;
        if received > cap {
            return Err(FetchError::TooLarge(received));
        }
        hasher.update(&chunk);
        file.write_all(&chunk).await.map_err(disk)?;
        progress(received);
    }
    file.flush().await.map_err(disk)?;
    drop(file);
    let found = crate::pin::hex(&hasher.finalize());
    if found != asset.sha256 {
        return Err(FetchError::Checksum {
            found,
            expected: asset.sha256.clone(),
        });
    }
    Ok(())
}

/// GETs a small file (the supported-Vibe manifest and its signature) into memory: `https` (or `http`
/// between loopback addresses: the tests' fake channel), redirects by the module's rule, at most `cap` bytes, within
/// `limit`. A missing file is [`FetchError::Status`] (404): the caller treats every error as "no offer".
///
/// # Errors
/// [`FetchError::Network`], [`FetchError::Status`], [`FetchError::TooLarge`], [`FetchError::Timeout`].
pub async fn get_small(url: &str, cap: u64, limit: Duration) -> Result<Vec<u8>, FetchError> {
    let inner = async {
        let parsed = reqwest::Url::parse(url).map_err(|e| FetchError::Network {
            host: url.to_owned(),
            detail: e.to_string(),
        })?;
        let loopback = matches!(parsed.host(), Some(url::Host::Ipv4(ip)) if ip.is_loopback())
            || matches!(parsed.host(), Some(url::Host::Ipv6(ip)) if ip.is_loopback());
        if !(parsed.scheme() == "https" || (parsed.scheme() == "http" && loopback)) {
            return Err(FetchError::Network {
                host: host_of(url),
                detail: "only https addresses are used".to_owned(),
            });
        }
        let client = vw_mistral::https_client().map_err(|e| FetchError::Network {
            host: host_of(url),
            detail: e.to_string(),
        })?;
        let response = follow(&client, url).await?;
        if !response.status().is_success() {
            return Err(FetchError::Status {
                host: host_of(response.url().as_str()),
                status: response.status().as_u16(),
            });
        }
        if let Some(length) = response.content_length()
            && length > cap
        {
            return Err(FetchError::TooLarge(length));
        }
        let mut body = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| network(url, &e))?;
            if (body.len() + chunk.len()) as u64 > cap {
                return Err(FetchError::TooLarge((body.len() + chunk.len()) as u64));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    };
    tokio::time::timeout(limit, inner)
        .await
        .unwrap_or(Err(FetchError::Timeout(limit)))
}

/// GETs `url`, following up to [`MAX_REDIRECTS`] redirects by the module's rule.
async fn follow(client: &reqwest::Client, url: &str) -> Result<reqwest::Response, FetchError> {
    let mut current = reqwest::Url::parse(url).map_err(|e| FetchError::Network {
        host: url.to_owned(),
        detail: e.to_string(),
    })?;
    for _ in 0..=MAX_REDIRECTS {
        let response = client
            .get(current.clone())
            .send()
            .await
            .map_err(|e| network(current.as_str(), &e))?;
        if !response.status().is_redirection() {
            return Ok(response);
        }
        let location = response
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| FetchError::Status {
                host: host_of(current.as_str()),
                status: response.status().as_u16(),
            })?;
        let next = current.join(location).map_err(|e| FetchError::Network {
            host: host_of(current.as_str()),
            detail: format!("bad redirect target: {e}"),
        })?;
        let loopback = |u: &reqwest::Url| {
            matches!(u.host(), Some(url::Host::Ipv4(ip)) if ip.is_loopback())
                || matches!(u.host(), Some(url::Host::Ipv6(ip)) if ip.is_loopback())
        };
        let allowed = next.scheme() == "https"
            || (next.scheme() == "http" && loopback(&current) && loopback(&next));
        if !allowed {
            return Err(FetchError::Network {
                host: host_of(next.as_str()),
                detail: format!("redirect to a non-https address refused: {}", next.scheme()),
            });
        }
        current = next;
    }
    Err(FetchError::Network {
        host: host_of(url),
        detail: format!("more than {MAX_REDIRECTS} redirects"),
    })
}

/// The uv executable's file name on this OS.
#[must_use]
pub const fn uv_name() -> &'static str {
    if cfg!(windows) { "uv.exe" } else { "uv" }
}

/// Takes the `uv` executable out of `archive` (`tar.gz` or `zip`) into `dir` and returns its path. Executable
/// bit set on Unix.
///
/// # Errors
/// [`FetchError::Archive`] (no single uv inside, unknown kind), [`FetchError::Disk`].
pub fn unpack_uv(archive: &Path, kind: &str, dir: &Path) -> Result<PathBuf, FetchError> {
    let bad = |what: &str| FetchError::Archive(what.to_owned());
    let disk = |path: &Path| {
        let path = path.to_path_buf();
        move |source| FetchError::Disk { path, source }
    };
    std::fs::create_dir_all(dir).map_err(disk(dir))?;
    let out = dir.join(uv_name());
    let file = std::fs::File::open(archive).map_err(disk(archive))?;
    let mut bytes = Vec::new();
    let mut found = 0_u32;
    match kind {
        "tar.gz" => {
            let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file));
            for entry in tar.entries().map_err(|e| bad(&e.to_string()))? {
                let mut entry = entry.map_err(|e| bad(&e.to_string()))?;
                let path = entry.path().map_err(|e| bad(&e.to_string()))?.into_owned();
                if entry.header().entry_type().is_file()
                    && path.file_name().is_some_and(|n| n == "uv")
                {
                    found += 1;
                    bytes.clear();
                    entry
                        .read_to_end(&mut bytes)
                        .map_err(|e| bad(&e.to_string()))?;
                }
            }
        }
        "zip" => {
            let mut zip = zip::ZipArchive::new(file).map_err(|e| bad(&e.to_string()))?;
            for index in 0..zip.len() {
                let mut entry = zip.by_index(index).map_err(|e| bad(&e.to_string()))?;
                let name = entry
                    .name()
                    .rsplit(['/', '\\'])
                    .next()
                    .unwrap_or_default()
                    .to_owned();
                if entry.is_file() && name.eq_ignore_ascii_case("uv.exe") {
                    found += 1;
                    bytes.clear();
                    entry
                        .read_to_end(&mut bytes)
                        .map_err(|e| bad(&e.to_string()))?;
                }
            }
        }
        other => return Err(bad(&format!("unknown archive kind {other}"))),
    }
    if found != 1 {
        return Err(bad(&format!(
            "{found} uv executables in the archive (expected 1)"
        )));
    }
    std::fs::write(&out, &bytes).map_err(disk(&out))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&out, std::fs::Permissions::from_mode(0o755))
            .map_err(disk(&out))?;
    }
    Ok(out)
}
