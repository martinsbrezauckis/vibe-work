//! Where a capture is written: `<session folder>/capture/<slug>-<YYYYMMDD-HHMMSS>/`, never anywhere else.
//! `capture/` holds a `.gitignore` of `*`, so the agent's `git add -A` cannot commit captures by accident;
//! the user still sees the files next to the project.
//!
//! Depends on: std, cap-std (directory handles). Called by: `capture`, tests.
//! Invariants (page content is hostile input: titles and URLs choose nothing but the slug):
//! - The slug is ASCII `a-z0-9-` only, at most [`SLUG_MAX`] characters, never empty (`page` then), so no separator,
//!   dot, drive letter, reserved device name or Unicode trick reaches a path.
//! - `capture/` must be a real folder whose resolved path is `<session folder>/capture` (a link or junction there is
//!   refused); the capture folder is created new (`create_dir` fails on any existing entry, also a planted link) and
//!   checked again after creation.
//! - Every new file uses `create_new` (`O_CREAT|O_EXCL`), so a planted link is not followed. After updates only the
//!   verified Before manifest via a create-new temporary file and rename over that entry, never through its link.
//! - Files are written, read and renamed through directory handles (`cap_std::fs::Dir`) of `capture/` and of the
//!   capture folder, opened right after their checks: a folder moved away and replaced by a
//!   link afterwards keeps receiving the writes (the handle follows the folder, not the name), and no path resolved
//!   through a handle may leave it (cap-std refuses `..` and links that point outside).
//!   Files use names from [`file_name`] (ours, or `NN-<ascii stem>.<ext>` for media) and bounded writes.
//! - Text files (Markdown, text, HTML, JSON, SVG) are written with every registered stored secret withheld
//!   (`vw_secrets::scrub`, the same forms the agent tools' answers hide): the agent reads these files with Vibe's own
//!   tools, past Vibe Work's answer scrub, and a page can copy a filled vault password into its text.

use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use cap_std::fs::{Dir, OpenOptions};

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
    /// Absolute path (for answers and messages; files go through `dir`).
    pub path: PathBuf,
    /// Path relative to the session folder (`capture/example-com-20261010-142233`).
    pub relative: String,
    /// The capture folder, opened after its checks.
    dir: Dir,
    /// `capture/`, opened after its checks (the staging folder of [`Folder::write_atomic`] lives there).
    base: Dir,
    written: Vec<Written>,
    total: u64,
}

/// Opens `leaf` inside `parent` as a directory handle: it must be a plain folder there (not a link), and the handle
/// cannot resolve outside `parent` (cap-std).
fn open_leaf(parent: &Dir, leaf: &str, shown: &Path) -> Result<Dir, String> {
    let meta = parent
        .symlink_metadata(leaf)
        .map_err(|e| format!("{}: {e}", shown.display()))?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Err(format!(
            "{} is not a plain folder; nothing was captured",
            shown.display()
        ));
    }
    parent
        .open_dir(leaf)
        .map_err(|e| format!("opening {}: {e}", shown.display()))
}

/// The session folder (canonical) and handles of it and of its checked `capture/` (created when `create`).
fn open_base(session: &Path, create: bool) -> Result<(PathBuf, Dir), String> {
    let session = session
        .canonicalize()
        .map_err(|e| format!("the session folder is not available: {e}"))?;
    let base = session.join("capture");
    real_dir(&base, &base, create)?;
    let root = Dir::open_ambient_dir(&session, cap_std::ambient_authority())
        .map_err(|e| format!("the session folder is not available: {e}"))?;
    let handle = open_leaf(&root, "capture", &base)?;
    Ok((base, handle))
}

/// `name` (ours: `/`-separated slug-safe parts) as a path relative to a folder handle; `None` for anything else.
fn relative_name(name: &str) -> Option<&Path> {
    let bad = name
        .split('/')
        .any(|p| p.is_empty() || p == "." || p == ".." || p.contains('\\') || p.contains(':'));
    (!bad).then(|| Path::new(name))
}

fn new_file(dir: &Dir, name: &Path) -> std::io::Result<cap_std::fs::File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    dir.open_with(name, &options)
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
fn keep_out_of_git(base: &Dir) {
    if let Ok(mut file) = new_file(base, Path::new(".gitignore")) {
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
        let (base, handle) = open_base(session, true)?;
        keep_out_of_git(&handle);
        let name = format!("{}-{stamp}", self::slug(slug));
        for n in 1..1000u32 {
            let leaf = if n == 1 {
                name.clone()
            } else {
                format!("{name}-{n}")
            };
            let path = base.join(&leaf);
            match handle.create_dir(&leaf) {
                Ok(()) => {
                    let dir = open_leaf(&handle, &leaf, &path)?;
                    return Ok(Self {
                        path,
                        relative: format!("capture/{leaf}"),
                        dir,
                        base: handle,
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
        let leaf = relative
            .strip_prefix("capture/")
            .filter(|l| created_leaf(l))
            .ok_or("not a capture folder")?;
        let (base, handle) = open_base(session, false)?;
        let path = base.join(leaf);
        let dir = open_leaf(&handle, leaf, &path)?;
        Ok(Self {
            path,
            relative: relative.to_owned(),
            dir,
            base: handle,
            written: Vec::new(),
            total: 0,
        })
    }

    /// Reads a file this capture folder holds (a regular file, not a link), at most `cap` bytes.
    ///
    /// # Errors
    /// A plain reason.
    pub fn read(&self, name: &str, cap: u64) -> Result<Vec<u8>, String> {
        let relative = relative_name(name).ok_or_else(|| format!("{name} is not a file name"))?;
        let meta = self
            .dir
            .symlink_metadata(relative)
            .map_err(|e| format!("{name}: {e}"))?;
        if !meta.is_file() || meta.len() > cap {
            return Err(format!("{name} is not a capture file"));
        }
        let file = self
            .dir
            .open(relative)
            .map_err(|e| format!("{name}: {e}"))?;
        let mut bytes = Vec::new();
        file.take(cap + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("{name}: {e}"))?;
        if bytes.len() as u64 > cap {
            return Err(format!("{name} is not a capture file"));
        }
        Ok(bytes)
    }

    /// A subfolder (`media`), created new inside the capture folder.
    ///
    /// # Errors
    /// A plain reason.
    pub fn subfolder(&self, name: &str) -> Result<PathBuf, String> {
        let leaf = slug(name);
        let path = self.path.join(&leaf);
        match self.dir.create_dir(&leaf) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(format!("creating {}: {e}", path.display())),
        }
        // A plain folder of this capture folder (a planted link there is refused).
        open_leaf(&self.dir, &leaf, &path)?;
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
        let relative = relative_name(name).ok_or_else(|| format!("{name} is not a file name"))?;
        let mut file =
            new_file(&self.dir, relative).map_err(|e| format!("creating {name}: {e}"))?;
        if let Err(e) = file.write_all(bytes).and_then(|()| file.sync_all()) {
            drop(file);
            let _ = self.dir.remove_file(relative);
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

    /// Publishes a complete file in one filesystem operation. Staging is outside this capture folder,
    /// so process death during the write never exposes a partly written result.
    ///
    /// # Errors
    /// The ordinary capture path, secret, size or exclusive-create checks fail.
    pub fn write_atomic(&mut self, name: &str, bytes: &[u8], cap: u64) -> Result<Written, String> {
        /// Removes the staging folder on every path.
        struct Pending<'a>(&'a Dir, String);
        impl Drop for Pending<'_> {
            fn drop(&mut self) {
                let _ = self.0.remove_dir_all(&self.1);
            }
        }
        let relative = relative_name(name).ok_or_else(|| format!("{name} is not a file name"))?;
        let leaf = self
            .path
            .file_name()
            .ok_or("capture has no name")?
            .to_string_lossy();
        let staging = format!(".pending-{leaf}");
        self.base
            .create_dir(&staging)
            .map_err(|e| format!("staging the capture: {e}"))?;
        let pending = Pending(&self.base, staging.clone());
        let staged_path = self.path.with_file_name(&staging);
        let staged_dir = open_leaf(&self.base, &staging, &staged_path)?;
        let mut staged = Self {
            path: staged_path,
            relative: self.relative.clone(),
            dir: staged_dir,
            base: self
                .base
                .try_clone()
                .map_err(|e| format!("staging the capture: {e}"))?,
            written: Vec::new(),
            total: self.total,
        };
        let written = staged.write(name, bytes, cap)?;
        staged
            .dir
            .hard_link(relative, &self.dir, relative)
            .map_err(|e| format!("saving {name}: {e}"))?;
        drop(staged);
        drop(pending);
        self.total += written.bytes;
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
        let temp = "capture.json.tmp";
        let current = self
            .dir
            .symlink_metadata(name)
            .map_err(|e| format!("checking {name}: {e}"))?;
        if !current.is_file() || current.len() > cap || self.read(name, cap)? != expected {
            return Err("capture.json is no longer the Before record".to_owned());
        }
        let scrubbed = vw_secrets::scrub(bytes);
        let bytes = scrubbed
            .as_ref()
            .map_or(bytes, |(clean, _)| clean.as_slice());
        let len = bytes.len() as u64;
        if len > cap || self.total + len > TOTAL_MAX {
            return Err("capture.json would exceed its capture cap".to_owned());
        }
        let mut file =
            new_file(&self.dir, Path::new(temp)).map_err(|e| format!("creating {temp}: {e}"))?;
        let result = file.write_all(bytes).and_then(|()| file.sync_all());
        drop(file);
        if let Err(e) = result {
            let _ = self.dir.remove_file(temp);
            return Err(format!("writing {temp}: {e}"));
        }
        // Recheck immediately before rename: refuse a static replacement, including a link or arbitrary file.
        let checked = self
            .dir
            .symlink_metadata(name)
            .is_ok_and(|meta| meta.is_file() && meta.len() <= cap)
            && self.read(name, cap).is_ok_and(|old| old == expected);
        if !checked {
            let _ = self.dir.remove_file(temp);
            return Err("capture.json changed before the After record was saved".to_owned());
        }
        if let Err(e) = self.dir.rename(temp, &self.dir, name) {
            let _ = self.dir.remove_file(temp);
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
