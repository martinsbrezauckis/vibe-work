//! Reading downloaded archives as hostile input: only the members the catalog lists are read
//! (or, for an npm tarball, every regular file under `package/`), each into memory with a size cap.
//!
//! Depends on: flate2, tar, zip. Called by: `install`.
//! Invariants:
//! - Nothing is written to disk here: the caller writes the returned bytes to paths the catalog chose (the archive's
//!   own paths are never used as destinations, except npm's, which are checked by [`crate::catalog_rules`]'s path
//!   rule before use).
//! - A listed member that is a link, a device or a folder is refused; a missing listed member is refused.
//! - npm mode refuses any member that is not a regular file or a folder, and any path with an empty, `.` or `..`
//!   part, an absolute path or a backslash.
//! - Per-member and total size caps hold even when a header lies about the size (reads are bounded by `take`).
//! - **The whole scan is bounded, including unlisted archive members**, not only the members we
//!   keep: at most [`ScanLimits::entries`] members are looked at and at most [`ScanLimits::bytes`] of the
//!   decompressed tar stream (headers, skipped members and padding included) is read, whichever comes first; then the
//!   archive is refused. A skill pack's tarball is not hash-pinned as a whole (GitHub regenerates it), so without this
//!   a hostile pack could keep the unpack step busy with millions of empty members or a gzip bomb of unlisted data.
//! - [`read_listed`] stops as soon as every listed member has been read. Later duplicates are never looked at, which
//!   is the same rule as before (the first copy of a name wins; the caller hashes exactly the bytes returned).
//! - Zip (only the Windows Node runtime, whose archive is SHA-256-pinned before it gets here): the member count is
//!   checked before any member is opened; members that are skipped are never decompressed by the zip crate.

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Read};
use std::rc::Rc;

use crate::catalog::ArchiveFormat;
use crate::error::MarketError;

/// Largest single member read (the Node binary is ~120 MB on Linux).
pub const MAX_MEMBER_BYTES: u64 = 200 * 1024 * 1024;
/// Largest total read from one archive.
pub const MAX_TOTAL_BYTES: u64 = 300 * 1024 * 1024;
/// Most members read from one npm tarball.
pub const MAX_NPM_MEMBERS: usize = 4000;

/// How much of one archive may be looked at, kept or not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScanLimits {
    /// Most members (files, folders, links) looked at.
    pub entries: usize,
    /// Most bytes of the decompressed tar stream read (headers, skipped data and padding included). Zip members that
    /// are skipped are not decompressed, so for a zip only kept members count (and [`MAX_TOTAL_BYTES`] already caps
    /// those).
    pub bytes: u64,
}

/// The limits every install uses. on the built-in catalog's real downloads: the largest is
/// Node 24.21.0's Linux x64 tarball, 5,888 members and 197.1 MiB of tar stream (the macOS and arm64 ones 5,888 /
/// 192-195 MiB; the Windows zips 2,459 members); the skill packs are at most 655 members / 24.4 MiB (context7);
/// playwright-core 115 members / 13.1 MiB. 20,000 members and 512 MiB leave 3.4x and 2.6x headroom for newer pins,
/// and 512 MiB of gzip output is about one to two seconds of CPU in a release build, so a hostile archive costs at
/// most that.
pub const SCAN_LIMITS: ScanLimits = ScanLimits {
    entries: 20_000,
    bytes: 512 * 1024 * 1024,
};

/// A reader that fails once more than `left` bytes were asked for, and remembers that it did (so the caller can say
/// "too large" instead of the tar crate's wrapped I/O error).
struct Capped<R> {
    inner: R,
    left: u64,
    over: Rc<Cell<bool>>,
}

impl<R: Read> Read for Capped<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.left == 0 {
            // One more byte decides: a stream that ends exactly at the cap is fine.
            let mut probe = [0_u8; 1];
            if self.inner.read(&mut probe)? == 0 {
                return Ok(0);
            }
            self.over.set(true);
            return Err(std::io::Error::other("scan cap reached"));
        }
        let room = usize::try_from(self.left)
            .unwrap_or(usize::MAX)
            .min(buf.len());
        let read = self.inner.read(&mut buf[..room])?;
        self.left -= read as u64;
        Ok(read)
    }
}

/// A tar over a capped gzip stream of in-memory bytes.
type CappedTar<'a> = tar::Archive<Capped<flate2::read::GzDecoder<Cursor<&'a [u8]>>>>;

/// A tar.gz reader over `bytes` whose decompressed stream is capped at `limits.bytes`; `over` says whether the cap
/// was what stopped it.
fn capped_tar(bytes: &[u8], limits: ScanLimits) -> (CappedTar<'_>, Rc<Cell<bool>>) {
    let over = Rc::new(Cell::new(false));
    let reader = Capped {
        inner: flate2::read::GzDecoder::new(Cursor::new(bytes)),
        left: limits.bytes,
        over: Rc::clone(&over),
    };
    (tar::Archive::new(reader), over)
}

/// Replaces any error by the scan-cap refusal when the cap is what stopped the read.
fn cap_wins<T>(
    result: Result<T, MarketError>,
    over: &Cell<bool>,
    limits: ScanLimits,
) -> Result<T, MarketError> {
    if over.get() {
        return Err(refuse(format!(
            "the archive unpacks to more than {} MiB",
            limits.bytes / (1024 * 1024)
        )));
    }
    result
}

fn too_many(limits: ScanLimits) -> MarketError {
    refuse(format!(
        "the archive holds more than {} members",
        limits.entries
    ))
}

fn refuse(message: impl Into<String>) -> MarketError {
    MarketError::Archive(message.into())
}

/// Reads exactly the members named in `wanted` from a tar.gz or zip in `bytes`. With `strip_top`, each archive path
/// loses its first part (GitHub archives hold one top folder `<repo>-<commit>/`).
///
/// # Errors
/// [`MarketError::Archive`] for a malformed archive, a listed member that is not a regular file, a missing listed
/// member, or a size cap.
pub fn read_listed(
    bytes: &[u8],
    format: ArchiveFormat,
    wanted: &[&str],
    strip_top: bool,
) -> Result<BTreeMap<String, Vec<u8>>, MarketError> {
    read_listed_within(bytes, format, wanted, strip_top, SCAN_LIMITS)
}

/// [`read_listed`] with explicit scan limits (the installer always uses [`SCAN_LIMITS`]; tests use small ones).
///
/// # Errors
/// As [`read_listed`], plus a refusal when the archive has more than `limits.entries` members or its tar stream is
/// longer than `limits.bytes` before every listed member was found.
pub fn read_listed_within(
    bytes: &[u8],
    format: ArchiveFormat,
    wanted: &[&str],
    strip_top: bool,
    limits: ScanLimits,
) -> Result<BTreeMap<String, Vec<u8>>, MarketError> {
    let distinct: BTreeSet<&str> = wanted.iter().copied().collect();
    let mut found = BTreeMap::new();
    let mut total = 0_u64;
    // Ok(true) when the member was listed and is now kept.
    let mut take = |path: String,
                    kind_ok: bool,
                    size: u64,
                    reader: &mut dyn Read|
     -> Result<bool, MarketError> {
        let key = if strip_top {
            match path.split_once('/') {
                Some((_, rest)) => rest.to_owned(),
                None => return Ok(false),
            }
        } else {
            path
        };
        if !distinct.contains(key.as_str()) || found.contains_key(&key) {
            return Ok(false);
        }
        if !kind_ok {
            return Err(refuse(format!("{key} is not a regular file")));
        }
        let data = bounded(reader, size, &key)?;
        total += data.len() as u64;
        if total > MAX_TOTAL_BYTES {
            return Err(refuse("the archive holds more than the size cap"));
        }
        found.insert(key, data);
        Ok(true)
    };
    let mut kept = 0_usize;
    match format {
        ArchiveFormat::TarGz => {
            let (mut archive, over) = capped_tar(bytes, limits);
            let mut scan = || -> Result<(), MarketError> {
                let mut seen = 0_usize;
                for entry in archive
                    .entries()
                    .map_err(|e| refuse(format!("not a tar.gz: {e}")))?
                {
                    let mut entry = entry.map_err(|e| refuse(format!("unreadable member: {e}")))?;
                    seen += 1;
                    if seen > limits.entries {
                        return Err(too_many(limits));
                    }
                    let path = String::from_utf8_lossy(&entry.path_bytes()).into_owned();
                    let kind_ok = entry.header().entry_type().is_file();
                    let size = entry.header().size().unwrap_or(u64::MAX);
                    if take(path, kind_ok, size, &mut entry)? {
                        kept += 1;
                        // Every listed member is in hand: nothing after this is read or decompressed.
                        if kept == distinct.len() {
                            break;
                        }
                    }
                }
                Ok(())
            };
            let scanned = scan();
            cap_wins(scanned, &over, limits)?;
        }
        ArchiveFormat::Zip => {
            let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
                .map_err(|e| refuse(format!("not a zip: {e}")))?;
            if archive.len() > limits.entries {
                return Err(too_many(limits));
            }
            for index in 0..archive.len() {
                let mut file = archive
                    .by_index(index)
                    .map_err(|e| refuse(format!("unreadable member: {e}")))?;
                let path = file.name().to_owned();
                let is_link = file.unix_mode().is_some_and(|m| m & 0o170_000 == 0o120_000);
                let kind_ok = file.is_file() && !is_link;
                let size = file.size();
                if take(path, kind_ok, size, &mut file)? {
                    kept += 1;
                    if kept == distinct.len() {
                        break;
                    }
                }
            }
        }
    }
    if let Some(missing) = wanted.iter().find(|w| !found.contains_key(**w)) {
        return Err(refuse(format!("{missing} is missing from the archive")));
    }
    Ok(found)
}

/// Reads every regular file under `package/` of an npm tarball, keyed by its path below `package/`.
///
/// # Errors
/// [`MarketError::Archive`] for a malformed tarball, a link or other special member, a bad path, or a cap.
pub fn read_npm(bytes: &[u8]) -> Result<BTreeMap<String, Vec<u8>>, MarketError> {
    read_npm_within(bytes, SCAN_LIMITS)
}

/// [`read_npm`] with explicit scan limits (the installer always uses [`SCAN_LIMITS`]; tests use small ones).
///
/// # Errors
/// As [`read_npm`], plus a refusal when the tarball has more than `limits.entries` members (folders count) or its
/// tar stream is longer than `limits.bytes`.
pub fn read_npm_within(
    bytes: &[u8],
    limits: ScanLimits,
) -> Result<BTreeMap<String, Vec<u8>>, MarketError> {
    let (mut archive, over) = capped_tar(bytes, limits);
    let scanned = npm_members(&mut archive, limits);
    cap_wins(scanned, &over, limits)
}

fn npm_members<R: Read>(
    archive: &mut tar::Archive<R>,
    limits: ScanLimits,
) -> Result<BTreeMap<String, Vec<u8>>, MarketError> {
    let mut out = BTreeMap::new();
    let mut folded = BTreeSet::new();
    let mut total = 0_u64;
    let mut seen = 0_usize;
    for entry in archive
        .entries()
        .map_err(|e| refuse(format!("not a tar.gz: {e}")))?
    {
        let mut entry = entry.map_err(|e| refuse(format!("unreadable member: {e}")))?;
        seen += 1;
        if seen > limits.entries {
            return Err(too_many(limits));
        }
        let raw = String::from_utf8_lossy(&entry.path_bytes()).into_owned();
        let kind = entry.header().entry_type();
        if kind.is_dir() {
            continue;
        }
        if !kind.is_file() {
            return Err(refuse(format!("{raw} is a link or special file")));
        }
        let Some(path) = raw.strip_prefix("package/") else {
            return Err(refuse(format!("{raw} is outside package/")));
        };
        crate::catalog_rules::check_path(path)
            .map_err(|_| refuse(format!("{raw} has a bad path")))?;
        // Two names that differ only in case would land on one file on a case-insensitive disk (Windows, macOS):
        // refused everywhere, so every OS installs the same files.
        if !folded.insert(path.to_lowercase()) {
            return Err(refuse(format!(
                "{raw} differs from another file only in letter case"
            )));
        }
        if out.len() >= MAX_NPM_MEMBERS {
            return Err(refuse("the package holds too many files"));
        }
        let size = entry.header().size().unwrap_or(u64::MAX);
        let data = bounded(&mut entry, size, path)?;
        total += data.len() as u64;
        if total > MAX_TOTAL_BYTES {
            return Err(refuse("the package holds more than the size cap"));
        }
        out.insert(path.to_owned(), data);
    }
    Ok(out)
}

/// Reads at most [`MAX_MEMBER_BYTES`] (and no more than the header claims plus one byte, so a lying header is caught).
fn bounded(reader: &mut dyn Read, claimed: u64, name: &str) -> Result<Vec<u8>, MarketError> {
    if claimed > MAX_MEMBER_BYTES {
        return Err(refuse(format!("{name} is larger than the size cap")));
    }
    let mut data = Vec::new();
    reader
        .take(MAX_MEMBER_BYTES + 1)
        .read_to_end(&mut data)
        .map_err(|e| refuse(format!("{name} could not be read: {e}")))?;
    if data.len() as u64 > MAX_MEMBER_BYTES {
        return Err(refuse(format!("{name} is larger than the size cap")));
    }
    Ok(data)
}
