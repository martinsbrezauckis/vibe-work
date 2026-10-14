//! Where a capture is written: `<session folder>/capture/<slug>-<YYYYMMDD-HHMMSS>/`, never anywhere else.
//! `capture/` holds a `.gitignore` of `*`, so the agent's `git add -A` cannot commit captures by accident;
//! the user still sees the files next to the project.
//!
//! Depends on: std. Called by: `capture`, tests.
//! Invariants (page content is hostile input: titles and URLs choose nothing but the slug):
//! - The slug is ASCII `a-z0-9-` only, at most [`SLUG_MAX`] characters, never empty (`page` then), so no separator,
//!   dot, drive letter, reserved device name or Unicode trick reaches a path.
//! - `capture/` must be a real folder whose resolved path is `<session folder>/capture` (a link or junction there is
//!   refused); the capture folder is created new (`create_dir` fails on any existing entry, also a planted link) and
//!   checked again after creation.
//! - Every new file uses `create_new` (`O_CREAT|O_EXCL`), so a planted link is not followed. After updates only the
//!   verified Before manifest via a create-new temporary file and rename over that entry, never through its link.
//!   Files use names from [`file_name`] (ours, or `NN-<ascii stem>.<ext>` for media) and bounded writes.
//! - Text files (Markdown, text, HTML, JSON, SVG) are written with every registered stored secret withheld
//!   (`vw_secrets::scrub`, the same forms the agent tools' answers hide): the agent reads these files with Vibe's own
//!   tools, past Vibe Work's answer scrub, and a page can copy a filled vault password into its text.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Longest slug.
pub const SLUG_MAX: usize = 48;
/// Most bytes one capture may write (all files together).
pub const TOTAL_MAX: u64 = 96 * 1024 * 1024;

/// An ASCII slug of `text` (`Hello, Wörld!` → `hello-w-rld`), at most [`SLUG_MAX`] characters, `page` when empty.
#[must_use]
pub fn slug(text: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            if dash && !out.is_empty() {
                out.push('-');
            }
            dash = false;
            out.push(c.to_ascii_lowercase());
        } else {
            dash = true;
        }
        if out.len() >= SLUG_MAX {
            break;
        }
    }
    out.truncate(SLUG_MAX);
    let out = out.trim_matches('-').to_owned();
    if out.is_empty() {
        "page".to_owned()
    } else {
        out
    }
}

/// A media file name: `NN-<slug of the stem, at most 32>.<ext>` (`ext` from the sniffed bytes, ours).
#[must_use]
pub fn file_name(index: usize, stem: &str, ext: &str) -> String {
    let mut stem = slug(stem);
    stem.truncate(32);
    let stem = stem.trim_end_matches('-');
    format!("{index:02}-{stem}.{ext}")
}

/// UTC civil date and time of `secs` since the epoch: (year, month, day, hour, minute, second).
#[must_use]
pub fn civil(secs: u64) -> (i64, u32, u32, u32, u32, u32) {
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    let rem = secs % 86_400;
    // Howard Hinnant's days-to-civil.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    let narrow = |v: i64| u32::try_from(v).unwrap_or(0);
    let narrow_u = |v: u64| u32::try_from(v).unwrap_or(0);
    (
        year,
        narrow(month),
        narrow(day),
        narrow_u(rem / 3600),
        narrow_u(rem % 3600 / 60),
        narrow_u(rem % 60),
    )
}

/// The capture time: (`20261010-142233` for folder names, `2026-10-10T14:22:33Z`).
#[must_use]
pub fn stamp(now: SystemTime) -> (String, String) {
    let secs = now.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let (y, mo, d, h, mi, s) = civil(secs);
    (
        format!("{y:04}{mo:02}{d:02}-{h:02}{mi:02}{s:02}"),
        format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z"),
    )
}

/// One file written.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Written {
    /// Path inside the capture folder (`media/01-logo.svg`).
    pub name: String,
    /// Bytes.
    pub bytes: u64,
}

/// A capture folder being written.
#[derive(Debug)]
pub struct Folder {
    /// Absolute path.
    pub path: PathBuf,
    /// Path relative to the session folder (`capture/example-com-20261010-142233`).
    pub relative: String,
    written: Vec<Written>,
    total: u64,
}

/// `dir` must be a real folder (created when missing) that resolves to `expected` itself.
fn real_dir(dir: &Path, expected: &Path, create: bool) -> Result<(), String> {
    match std::fs::symlink_metadata(dir) {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(format!("{} is a link; nothing was captured", dir.display()));
        }
        Ok(meta) if !meta.is_dir() => {
            return Err(format!(
                "{} is not a folder; nothing was captured",
                dir.display()
            ));
        }
        Ok(_) => {}
        Err(_) if create => {
            std::fs::create_dir(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
        }
        Err(e) => return Err(format!("{}: {e}", dir.display())),
    }
    let resolved = dir.canonicalize().map_err(|e| e.to_string())?;
    if resolved == expected {
        Ok(())
    } else {
        Err(format!(
            "{} resolves somewhere else; nothing was captured",
            dir.display()
        ))
    }
}

/// The `.gitignore` written into `capture/`: the folder ignores itself.
pub const GITIGNORE: &str = "# Vibe Work: captures stay out of commits (this folder ignores itself). To commit a capture, \
                             add it with git add -f, or delete this file.\n*\n";

/// Makes `capture/` ignore itself for git (so the agent's `git add -A` cannot commit captures): a new
/// `capture/.gitignore` holding `*`, written once and never over an existing entry (a user's own file stays; a
/// planted link is not followed: `create_new`). Not the user's `.gitignore` and not `.git/info/exclude` (shared by
/// every copy of the repository). Best effort: a capture is not refused when it cannot be written.
fn keep_out_of_git(base: &Path) {
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(base.join(".gitignore"));
    if let Ok(mut file) = file {
        let _ = file.write_all(GITIGNORE.as_bytes());
    }
}

/// A leaf that `create` can make with the UTC stamp format from [`stamp`].
fn created_leaf(leaf: &str) -> bool {
    fn base(leaf: &str) -> bool {
        let Some((part, time)) = leaf.rsplit_once('-') else {
            return false;
        };
        // The final hyphen belongs to HHMMSS; the date starts eight bytes before it.
        let Some((name, date)) = part.rsplit_once('-') else {
            return false;
        };
        if name.is_empty() || name.len() > SLUG_MAX || slug(name) != name {
            return false;
        }
        if date.len() != 8
            || time.len() != 6
            || !date.bytes().chain(time.bytes()).all(|b| b.is_ascii_digit())
        {
            return false;
        }
        let number = |s: &str| s.parse::<u32>().unwrap_or(0);
        let (year, month, day) = (number(&date[..4]), number(&date[4..6]), number(&date[6..8]));
        let is_leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
        let days = match month {
            2 => {
                if is_leap {
                    29
                } else {
                    28
                }
            }
            4 | 6 | 9 | 11 => 30,
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            _ => return false,
        };
        year >= 1970
            && day >= 1
            && day <= days
            && number(&time[..2]) < 24
            && number(&time[2..4]) < 60
            && number(&time[4..6]) < 60
    }
    if base(leaf) {
        return true;
    }
    let Some((name, suffix)) = leaf.rsplit_once('-') else {
        return false;
    };
    suffix
        .parse::<u32>()
        .is_ok_and(|n| (2..1000).contains(&n) && suffix == n.to_string())
        && base(name)
}

impl Folder {
    /// Creates `<session>/capture/<slug>-<stamp>[-N]/` (a new folder; `-2`, `-3`... when the name exists).
    ///
    /// # Errors
    /// A plain reason: the session folder is missing, `capture` is a link or not a folder, nothing could be created.
    pub fn create(session: &Path, slug: &str, stamp: &str) -> Result<Self, String> {
        let session = session
            .canonicalize()
            .map_err(|e| format!("the session folder is not available: {e}"))?;
        let base = session.join("capture");
        real_dir(&base, &base, true)?;
        keep_out_of_git(&base);
        let name = format!("{}-{stamp}", self::slug(slug));
        for n in 1..1000u32 {
            let leaf = if n == 1 {
                name.clone()
            } else {
                format!("{name}-{n}")
            };
            let path = base.join(&leaf);
            match std::fs::create_dir(&path) {
                Ok(()) => {
                    real_dir(&path, &path, false)?;
                    return Ok(Self {
                        path,
                        relative: format!("capture/{leaf}"),
                        written: Vec::new(),
                        total: 0,
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(format!("creating the capture folder: {e}")),
            }
        }
        Err("no free capture folder name".to_owned())
    }

    /// Opens an existing capture folder again (the `after` shot writes next to its `before`): it must still be a real
    /// folder inside `<session>/capture`.
    ///
    /// # Errors
    /// A plain reason.
    pub fn reopen(session: &Path, relative: &str) -> Result<Self, String> {
        let session = session
            .canonicalize()
            .map_err(|e| format!("the session folder is not available: {e}"))?;
        let base = session.join("capture");
        real_dir(&base, &base, false)?;
        let leaf = relative
            .strip_prefix("capture/")
            .filter(|l| created_leaf(l))
            .ok_or("not a capture folder")?;
        let path = base.join(leaf);
        real_dir(&path, &path, false)?;
        Ok(Self {
            path,
            relative: relative.to_owned(),
            written: Vec::new(),
            total: 0,
        })
    }

    /// Reads a file this capture folder holds (a regular file, not a link), at most `cap` bytes.
    ///
    /// # Errors
    /// A plain reason.
    pub fn read(&self, name: &str, cap: u64) -> Result<Vec<u8>, String> {
        let path = self.path.join(name);
        let meta = std::fs::symlink_metadata(&path).map_err(|e| format!("{name}: {e}"))?;
        if !meta.is_file() || meta.len() > cap {
            return Err(format!("{name} is not a capture file"));
        }
        std::fs::read(&path).map_err(|e| format!("{name}: {e}"))
    }

    /// A subfolder (`media`), created new inside the capture folder.
    ///
    /// # Errors
    /// A plain reason.
    pub fn subfolder(&self, name: &str) -> Result<PathBuf, String> {
        let path = self.path.join(slug(name));
        real_dir(&path, &path, true)?;
        Ok(path)
    }

    /// Writes `bytes` as a new file `name` (a path inside the capture folder made of [`slug`]-safe parts: ours),
    /// at most `cap` bytes. Returns the file's relative name.
    ///
    /// # Errors
    /// A plain reason (over a cap, the name exists, I/O); a partly written file is removed.
    pub fn write(&mut self, name: &str, bytes: &[u8], cap: u64) -> Result<Written, String> {
        let text = [".md", ".txt", ".html", ".json", ".svg", ".css"]
            .iter()
            .any(|ext| name.to_ascii_lowercase().ends_with(ext));
        let scrubbed = if text { vw_secrets::scrub(bytes) } else { None };
        let bytes = scrubbed
            .as_ref()
            .map_or(bytes, |(clean, _)| clean.as_slice());
        let len = bytes.len() as u64;
        if len > cap {
            return Err(format!(
                "{name} would be {len} bytes, over its cap of {cap}"
            ));
        }
        if self.total + len > TOTAL_MAX {
            return Err(format!(
                "{name} would pass the capture's total cap of {TOTAL_MAX} bytes"
            ));
        }
        if name
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == ".." || p.contains('\\'))
        {
            return Err(format!("{name} is not a file name"));
        }
        let path = self.path.join(name);
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| format!("creating {name}: {e}"))?;
        if let Err(e) = file.write_all(bytes).and_then(|()| file.sync_all()) {
            drop(file);
            let _ = std::fs::remove_file(&path);
            return Err(format!("writing {name}: {e}"));
        }
        self.total += len;
        let written = Written {
            name: name.to_owned(),
            bytes: len,
        };
        self.written.push(written.clone());
        Ok(written)
    }

    /// Replaces only a manifest whose current bytes still match the checked Before record. The temporary file is
    /// new, and rename replaces the directory entry rather than opening its target if a link was planted there.
    ///
    /// # Errors
    /// A plain reason; on failure the old manifest stays untouched.
    pub fn rewrite_manifest(
        &mut self,
        expected: &[u8],
        bytes: &[u8],
        cap: u64,
    ) -> Result<(), String> {
        let name = "capture.json";
        let path = self.path.join(name);
        let current =
            std::fs::symlink_metadata(&path).map_err(|e| format!("checking {name}: {e}"))?;
        if !current.is_file() || current.len() > cap || self.read(name, cap)? != expected {
            return Err("capture.json is no longer the Before record".to_owned());
        }
        let temp = self.path.join("capture.json.tmp");
        let scrubbed = vw_secrets::scrub(bytes);
        let bytes = scrubbed
            .as_ref()
            .map_or(bytes, |(clean, _)| clean.as_slice());
        let len = bytes.len() as u64;
        if len > cap || self.total + len > TOTAL_MAX {
            return Err("capture.json would exceed its capture cap".to_owned());
        }
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|e| format!("creating capture.json.tmp: {e}"))?;
        let result = file.write_all(bytes).and_then(|()| file.sync_all());
        drop(file);
        if let Err(e) = result {
            let _ = std::fs::remove_file(&temp);
            return Err(format!("writing capture.json.tmp: {e}"));
        }
        // Recheck immediately before rename: refuse a static replacement, including a link or arbitrary file.
        let checked = std::fs::symlink_metadata(&path)
            .is_ok_and(|meta| meta.is_file() && meta.len() <= cap)
            && self.read(name, cap).is_ok_and(|old| old == expected);
        if !checked {
            let _ = std::fs::remove_file(&temp);
            return Err("capture.json changed before the After record was saved".to_owned());
        }
        if let Err(e) = std::fs::rename(&temp, &path) {
            let _ = std::fs::remove_file(&temp);
            return Err(format!("replacing capture.json: {e}"));
        }
        self.total += len;
        self.written.push(Written {
            name: name.to_owned(),
            bytes: len,
        });
        Ok(())
    }

    /// Every file written so far.
    #[must_use]
    pub fn written(&self) -> &[Written] {
        &self.written
    }

    /// Bytes written so far.
    #[must_use]
    pub fn total(&self) -> u64 {
        self.total
    }
}
