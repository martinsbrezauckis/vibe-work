//! Read-only look at Chrome's own files, so the setup can say "Turn on in Chrome" when the extension is off (the
//! person turned it off, or Chrome did after an update asked for new permissions), "Enable in Chrome" while Chrome
//! still holds a newly added extension for the person's consent, and name the installed extension version.
//!
//! Invariants (the probe only ever lowers a label; it never reports Connected and never writes):
//! - The registered user data folder and every child down to each read are real (no symlink, junction or
//!   reparse point), opened through verified parent handles. Files: `Secure Preferences`, then `Preferences`.
//! - Unix: `lstat` must show a regular file, else the file is skipped before it is opened; it is opened
//!   `O_RDONLY|O_NOFOLLOW|O_NONBLOCK|O_CLOEXEC` (never blocks, even on a FIFO), and `fstat` must show a regular file.
//!   Windows: opened with `FILE_FLAG_OPEN_REPARSE_POINT` and full sharing; the handle's information must show a
//!   disk file without directory or reparse attributes (a pipe or device has no such information and is refused).
//! - At most 16 MiB is read (one byte more refuses the file); JSON nests at most 128 deep (serde's limit). Only
//!   `extensions.settings.<id>.disable_reasons` and unpacked `location` are read: a non-zero number or a non-empty list means off; Chrome's
//!   external-extension reason (bit 13, value 8192, as a bit or a list item) means it still waits for consent.
//! - At most one probe runs, on its own thread. A caller waits at most 500 ms; while one runs, a caller gets the last
//!   result (or Unknown) and no second probe starts.
//! - Only a positive (off or waiting for consent) result is cached, for 5 s at most, keyed by path, size,
//!   modification time and file id.
//!
//! Depends on: std, `serde_json`, cap-std/cap-fs-ext and search's no-follow directory walk.
//! Called by: `addon`, `installation`, `setup` and external-registration reads.
use crate::search_walk::directory;
use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt, OpenOptionsSyncExt};
use cap_std::fs::{Dir, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime};

/// Largest preferences file read.
pub(super) const MAX_BYTES: u64 = 16 * 1024 * 1024;
/// How long a caller waits for a running probe.
pub(super) const WAIT: Duration = Duration::from_millis(500);
/// How long an "off" answer is reused.
pub(super) const POSITIVE_FOR: Duration = Duration::from_secs(5);
const FILES: [&str; 2] = ["Secure Preferences", "Preferences"];
/// Chrome's `DISABLE_EXTERNAL_EXTENSION` reason: an extension added from outside Chrome (the per-user registration)
/// stays off until the person accepts Chrome's "added" prompt. Chrome writes `"disable_reasons": [8192]` then, and
/// `[1]` (turned off by the person) once they turn an enabled extension off.
const EXTERNAL_EXTENSION: u64 = 1 << 13;

/// What Chrome's files say about the extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Probe {
    /// The extension is off in a profile (the person or Chrome turned it off).
    Off,
    /// Chrome added the extension from outside and holds it off until the person enables it in Chrome's prompt.
    AwaitingEnable,
    /// Chrome's files name the extension and say it is not off.
    NotOff,
    /// Nothing could be read, or the files do not say (the generic label).
    Unknown,
}

/// The identity of one file read: a changed file has a new key.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Key {
    path: PathBuf,
    size: u64,
    modified: Option<SystemTime>,
    id: u64,
}

#[derive(Default)]
struct State {
    running: bool,
    profiles: Vec<PathBuf>,
    extension_id: String,
    last: Option<(Probe, Instant)>,
    /// The cached positive result, its file key and when it was read.
    positive: Option<(Key, Instant, Probe)>,
    /// Tests: how many probe threads started.
    #[cfg(test)]
    started: usize,
    /// Tests: how long each probe thread waits before it reads.
    #[cfg(test)]
    delay: Duration,
}

impl State {
    fn recent(&self, now: Instant) -> Probe {
        self.last
            .filter(|(answer, read)| {
                // Both positive answers (off, awaiting Enable) expire like the cached positive read.
                matches!(answer, Probe::NotOff | Probe::Unknown)
                    || now.saturating_duration_since(*read) < POSITIVE_FOR
            })
            .map_or(Probe::Unknown, |(answer, _)| answer)
    }
}

/// One prober per Chrome connection state.
#[derive(Default, Clone)]
pub(crate) struct Prober {
    state: Arc<Mutex<State>>,
}

/// Ancestors above the registered folder may be OS aliases (macOS /var); the folder itself may not.
fn user_data_dir(path: &Path) -> Option<Dir> {
    // Remove trailing separators/dots so no-follow applies to the registered folder itself.
    let path: PathBuf = path.components().collect();
    let parent = Dir::open_ambient_dir(path.parent()?, cap_std::ambient_authority()).ok()?;
    directory(&parent, Path::new(path.file_name()?)).ok()
}

fn profile_dir(profile: &Path) -> Option<Dir> {
    let user_data = user_data_dir(profile.parent()?)?;
    directory(&user_data, Path::new(profile.file_name()?)).ok()
}

fn extension_dir(profile: &Dir, extension_id: &str) -> Option<Dir> {
    directory(profile, &Path::new("Extensions").join(extension_id)).ok()
}

/// Real profile folders with a Store copy or Chrome's own record of an unpacked copy.
pub(crate) fn profiles(user_data: &Path, extension_id: &str) -> Vec<PathBuf> {
    let Some(root) = user_data_dir(user_data) else {
        return Vec::new();
    };
    let Ok(entries) = root.entries() else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name();
            let profile = directory(&root, Path::new(&name)).ok()?;
            if extension_dir(&profile, extension_id).is_none()
                && !unpacked_profile(&profile, &user_data.join(&name), extension_id)
            {
                return None;
            }
            Some(user_data.join(name))
        })
        .collect();
    found.sort();
    found
}

/// Unpacked copies have no Extensions/<id> folder. Only read Chrome's bounded metadata;
/// never follow the unpacked path or treat its presence as a pairing grant.
fn unpacked_profile(profile: &Dir, path: &Path, extension_id: &str) -> bool {
    if let Ok(meta) = profile.symlink_metadata("Extensions") {
        if linked(&meta) || !meta.is_dir() {
            return false;
        }
        let Ok(extensions) = directory(profile, Path::new("Extensions")) else {
            return false;
        };
        if extensions
            .symlink_metadata(extension_id)
            .is_ok_and(|meta| linked(&meta) || !meta.is_dir())
        {
            return false;
        }
    }
    for name in FILES {
        if let Some((file, _)) = open_regular_at(profile, &path.join(name)) {
            match read_entry(file, extension_id) {
                Ok(None) => {}
                // Chromium ManifestLocation: unpacked UI installation or --load-extension.
                Ok(Some(entry)) => return matches!(entry["location"].as_u64(), Some(4 | 8)),
                Err(()) => return false,
            }
        }
    }
    false
}

/// The newest installed extension version in a profile (`Extensions/<id>/<version>_<n>`), read from folder names.
pub(crate) fn installed_version(profile: &Path, extension_id: &str) -> Option<String> {
    let profile = profile_dir(profile)?;
    let extension = extension_dir(&profile, extension_id)?;
    let entries = extension.entries().ok()?;
    entries
        .flatten()
        .filter_map(|entry| {
            directory(&extension, Path::new(&entry.file_name())).ok()?;
            let name = entry.file_name().into_string().ok()?;
            let (version, _) = name.rsplit_once('_')?;
            vw_vibe_install::support::compare_versions(version, version).map(|_| version.to_owned())
        })
        .max_by(|a, b| {
            vw_vibe_install::support::compare_versions(a, b).unwrap_or(std::cmp::Ordering::Equal)
        })
}

/// Opens `path` for reading only if it is a regular file, without following links or blocking.
fn open_regular(path: &Path) -> Option<(std::fs::File, Key)> {
    let parent = user_data_dir(path.parent()?)?;
    open_regular_at(&parent, path)
}

fn open_regular_at(parent: &Dir, path: &Path) -> Option<(std::fs::File, Key)> {
    let before = parent.symlink_metadata(path.file_name()?).ok()?;
    if !before.is_file() || linked(&before) {
        return None;
    }
    let file = open_no_follow(parent, Path::new(path.file_name()?))?;
    let meta = file.metadata().ok()?;
    if !meta.is_file() {
        return None;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const DIRECTORY: u32 = 0x10;
        const REPARSE: u32 = 0x400;
        if meta.file_attributes() & (DIRECTORY | REPARSE) != 0 {
            return None;
        }
    }
    #[cfg(unix)]
    let id = std::os::unix::fs::MetadataExt::ino(&meta);
    #[cfg(windows)]
    let id = std::os::windows::fs::MetadataExt::creation_time(&meta);
    #[cfg(not(any(unix, windows)))]
    let id = 0;
    let key = Key {
        path: path.to_owned(),
        size: meta.len(),
        modified: meta.modified().ok(),
        id,
    };
    Some((file, key))
}

fn linked(meta: &cap_std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use cap_std::fs::MetadataExt;
        meta.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    meta.file_type().is_symlink()
}

fn open_no_follow(parent: &Dir, name: &Path) -> Option<std::fs::File> {
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No).nonblock(true);
    parent
        .open_with(name, &options)
        .ok()
        .map(cap_std::fs::File::into_std)
}

/// Bounded bytes from a regular file, with the same no-follow open as preferences.
pub(super) fn read_regular(path: &Path, limit: u64) -> Option<Vec<u8>> {
    use std::io::Read;
    let (file, _) = open_regular(path)?;
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 <= limit).then_some(bytes)
}

/// What one opened file says; `None` when it does not name the extension, `Err` for a file that cannot be read or
/// parsed (the probe answers Unknown).
fn read_entry(file: std::fs::File, extension_id: &str) -> Result<Option<serde_json::Value>, ()> {
    use std::io::Read;
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(());
    }
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| ())?;
    Ok(value
        .get("extensions")
        .and_then(|e| e.get("settings"))
        .and_then(|s| s.get(extension_id))
        .cloned())
}

fn read_file(file: std::fs::File, extension_id: &str) -> Result<Option<Probe>, ()> {
    let Some(entry) = read_entry(file, extension_id)? else {
        return Ok(None);
    };
    let (off, external) = match entry.get("disable_reasons") {
        None | Some(serde_json::Value::Null) => (false, false),
        Some(serde_json::Value::Number(n)) => (
            n.as_u64() != Some(0),
            n.as_u64()
                .is_some_and(|bits| bits & EXTERNAL_EXTENSION != 0),
        ),
        Some(serde_json::Value::Array(list)) => (
            !list.is_empty(),
            list.iter()
                .any(|reason| reason.as_u64() == Some(EXTERNAL_EXTENSION)),
        ),
        Some(_) => return Err(()),
    };
    Ok(Some(match (off, external) {
        (false, _) => Probe::NotOff,
        (true, true) => Probe::AwaitingEnable,
        (true, false) => Probe::Off,
    }))
}

/// One probe of every profile's files (on the probe thread). The first file that names the extension decides for
/// its profile; the first profile where it is off (or waits for consent) gives the answer.
fn probe_once(profiles: &[PathBuf], extension_id: &str) -> (Probe, Option<Key>) {
    let mut answer = Probe::Unknown;
    for profile in profiles {
        let Some(dir) = profile_dir(profile) else {
            continue;
        };
        if extension_dir(&dir, extension_id).is_none()
            && !unpacked_profile(&dir, profile, extension_id)
        {
            continue;
        }
        for name in FILES {
            let path = profile.join(name);
            let Some((file, key)) = open_regular_at(&dir, &path) else {
                continue;
            };
            match read_file(file, extension_id) {
                Err(()) => return (Probe::Unknown, None),
                Ok(None) => {}
                Ok(Some(Probe::NotOff)) => {
                    answer = Probe::NotOff;
                    break;
                }
                Ok(Some(positive)) => return (positive, Some(key)),
            }
        }
    }
    (answer, None)
}

fn key_now(key: &Key, extension_id: &str) -> Option<Key> {
    let profile = profile_dir(key.path.parent()?)?;
    if extension_dir(&profile, extension_id).is_none()
        && !unpacked_profile(&profile, key.path.parent()?, extension_id)
    {
        return None;
    }
    open_regular_at(&profile, &key.path).map(|(_, key)| key)
}

impl Prober {
    /// What Chrome's files say now, at `now` (the cache clock).
    pub(crate) fn probe(&self, profiles: Vec<PathBuf>, extension_id: &str, now: Instant) -> Probe {
        let started = Instant::now();
        let receiver = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            if state.profiles != profiles || state.extension_id != extension_id {
                // A worker or positive result for another profile cannot diagnose this selection.
                if state.running {
                    return Probe::Unknown;
                }
                state.profiles.clone_from(&profiles);
                extension_id.clone_into(&mut state.extension_id);
                state.last = None;
                state.positive = None;
            }
            if let Some((key, read, answer)) = &state.positive {
                if now.duration_since(*read) < POSITIVE_FOR
                    && key_now(key, extension_id).as_ref() == Some(key)
                {
                    return *answer;
                }
                state.positive = None;
            }
            if state.running {
                return state.recent(now);
            }
            state.running = true;
            #[cfg(test)]
            {
                state.started += 1;
            }
            #[cfg(test)]
            let delay = state.delay;
            let (send, receive) = std::sync::mpsc::channel();
            let shared = self.state.clone();
            let id = extension_id.to_owned();
            let spawned = std::thread::Builder::new()
                .name("chrome-probe".into())
                .spawn(move || {
                    #[cfg(test)]
                    std::thread::sleep(delay);
                    let (answer, key) = probe_once(&profiles, &id);
                    let mut state = shared.lock().unwrap_or_else(PoisonError::into_inner);
                    state.running = false;
                    let read = now;
                    state.last = Some((answer, read));
                    if let Some(key) = key {
                        state.positive = Some((key, read, answer));
                    }
                    drop(state);
                    let _ = send.send(answer);
                });
            if spawned.is_err() {
                state.running = false;
                return Probe::Unknown;
            }
            receive
        };
        receiver.recv_timeout(WAIT).unwrap_or_else(|_| {
            self.state
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .recent(now + started.elapsed())
        })
    }

    #[cfg(test)]
    pub(super) fn slow_for_test(&self, delay: Duration) {
        self.state.lock().unwrap().delay = delay;
    }

    #[cfg(test)]
    pub(super) fn started_for_test(&self) -> usize {
        self.state.lock().unwrap().started
    }
}

/// Profile observation through the platform permission boundary.
pub(super) fn profiles_for(
    config: &super::setup::ChromeControlConfig,
    user_data: &Path,
    extension_id: &str,
) -> Vec<PathBuf> {
    if config.is_macos() {
        return Vec::new();
    }
    profiles_for_names(config, user_data, extension_id)
}

/// [`profiles_for`] for the profile-name search, which also reads on macOS once `browser_data::readable` allowed
/// this folder.
pub(super) fn profiles_for_names(
    config: &super::setup::ChromeControlConfig,
    user_data: &Path,
    extension_id: &str,
) -> Vec<PathBuf> {
    if super::setup::files::access(config, super::setup::files::Access::ReadDir, user_data).is_err()
    {
        return Vec::new();
    }
    profiles(user_data, extension_id)
}

/// What Chrome's files say about the extension in one profile folder, read now (no cache, no thread): the
/// Settings row of a paired profile whose folder is proven.
pub(super) fn folder(
    config: &super::setup::ChromeControlConfig,
    profile: &Path,
    extension_id: &str,
) -> Probe {
    if config.is_macos()
        || super::setup::files::access(config, super::setup::files::Access::ReadDir, profile)
            .is_err()
    {
        return Probe::Unknown;
    }
    probe_once(&[profile.to_path_buf()], extension_id).0
}

pub(super) fn version_for(
    config: &super::setup::ChromeControlConfig,
    profile: &Path,
    extension_id: &str,
) -> Option<String> {
    if config.is_macos() {
        return None;
    }
    super::setup::files::access(config, super::setup::files::Access::ReadDir, profile).ok()?;
    installed_version(profile, extension_id)
}

impl Prober {
    pub(super) fn probe_for(
        &self,
        config: &super::setup::ChromeControlConfig,
        profiles: Vec<PathBuf>,
        extension_id: &str,
        now: Instant,
    ) -> Probe {
        if config.is_macos() {
            return Probe::Unknown;
        }
        for profile in &profiles {
            if super::setup::files::access(config, super::setup::files::Access::ReadDir, profile)
                .is_err()
            {
                return Probe::Unknown;
            }
        }
        self.probe(profiles, extension_id, now)
    }
}

#[cfg(test)]
#[path = "probe_tests.rs"]
mod tests;
