//! Shared search/document/upload exclusion for app state and every known vault location.
//! Uses the folder resolver and opened-file paths; callers check before reading any bytes.
use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use vw_agent_tools::folders::{Layout, resolve};

use crate::core::Shared;

const PROTECTED: &str = vw_agent_tools::folders::PROTECTED_IDENTITY_REASON;
#[cfg(unix)]
type Identity = (u64, u64);
#[cfg(not(unix))]
type Identity = vw_platform::final_path::FileIdentity;
#[cfg(unix)]
const MAX_IDENTITY_ENTRIES: usize = 100_000;
#[cfg(unix)]
const IDENTITY_BUDGET: Duration = Duration::from_secs(3);
// Windows still needs opened handles for volume/file indices; retain its existing bounds.
#[cfg(not(unix))]
const MAX_IDENTITY_ENTRIES: usize = 4096;
#[cfg(not(unix))]
const IDENTITY_BUDGET: Duration = Duration::from_millis(100);
// Native hard-link checks run before Vibe reads or writes; keep their Windows metadata walk bounded separately.
#[cfg(windows)]
const NATIVE_IDENTITY_BUDGET: Duration = Duration::from_secs(1);

pub(crate) struct ReadExclusions {
    #[cfg(test)]
    clock: Option<std::sync::Arc<dyn Fn() -> Instant + Send + Sync>>,
    #[cfg(test)]
    entry_limit: Option<usize>,
    data: Option<PathBuf>,
    own: Option<PathBuf>,
    vaults: Vec<PathBuf>,
}

pub(crate) fn under(path: &Path, root: &Path) -> bool {
    vw_procmon::normalize(path).starts_with(vw_procmon::normalize(root))
}

impl ReadExclusions {
    pub(crate) fn for_chat(shared: &Shared, cwd: &Path) -> Result<Self, String> {
        Self::new(
            &shared.instructions.own_home,
            cwd,
            shared.config.keys.as_ref().map(|k| k.vault_profile_dir()),
            &vw_vault::location::ProfileInput::from_env(),
        )
    }

    pub(crate) fn new(
        home: &Path,
        cwd: &Path,
        selected: Option<PathBuf>,
        input: &vw_vault::location::ProfileInput,
    ) -> Result<Self, String> {
        let layout = Layout::new(home, cwd).map_err(|e| e.to_string())?;
        let data = home
            .parent()
            .map(resolve)
            .transpose()
            .map_err(|e| e.to_string())?;
        let mut paths = Vec::new();
        paths.extend(selected);
        if let Some(data) = &data {
            paths.push(
                vw_vault::private_location::private_vault_dir(data).map_err(|e| e.to_string())?,
            );
        }
        if let Ok(shared) = vw_vault::location::resolve(input) {
            paths.extend([shared.canonical, shared.legacy]);
        }
        let vaults = paths
            .iter()
            .map(|p| resolve(p).map_err(|e| e.to_string()))
            .collect::<Result<_, _>>()?;
        Ok(Self {
            #[cfg(test)]
            clock: None,
            #[cfg(test)]
            entry_limit: None,
            data,
            own: layout.own,
            vaults,
        })
    }

    pub(crate) fn excludes(&self, path: &Path) -> bool {
        self.vaults.iter().any(|v| under(path, v))
            || self.data.as_ref().is_some_and(|d| under(path, d))
                && !self.own.as_ref().is_some_and(|c| under(path, c))
    }

    /// True when `folder` is inside a protected root or holds one (the vault's container, the folder above the app
    /// data, a home folder): a place handed to the agent as a whole must not contain the vault or app state.
    pub(crate) fn overlaps(&self, folder: &Path) -> bool {
        self.excludes(folder)
            || self
                .vaults
                .iter()
                .chain(self.data.iter())
                .any(|root| under(root, folder))
    }

    /// Validate the opened object, not just the spelling checked before opening it. Refuse a replacement
    /// between resolution and open; the same handle supplies all bytes after this check.
    pub(crate) fn check_opened(
        &self,
        file: &File,
        named: &Path,
        deadline: Option<Instant>,
    ) -> Result<(), String> {
        let changed =
            || "The file's location could not be confirmed. Choose the file again.".to_owned();
        let opened = vw_platform::final_path::final_path(file).map_err(|_| changed())?;
        if self.excludes(&opened) || self.excludes(named) {
            return Err(PROTECTED.into());
        }
        let (_, links) =
            vw_platform::final_path::identity(file).map_err(|_| PROTECTED.to_owned())?;
        if links > 1 {
            self.check_hard_links(file, deadline)
                .map_err(|()| PROTECTED.to_owned())?;
        }
        let real = named.canonicalize().map_err(|_| changed())?;
        if self.excludes(&real) {
            return Err(PROTECTED.into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let at = std::fs::metadata(&real).map_err(|_| changed())?;
            let actual = file.metadata().map_err(|_| changed())?;
            if at.dev() != actual.dev() || at.ino() != actual.ino() {
                return Err(changed());
            }
            #[cfg(target_os = "macos")]
            if opened != real && links > 1 && actual.is_file() {
                // F_GETPATH may name another hard link. Both names must still identify this handle;
                // the bounded protected-identity scan above must have cleared all of its aliases.
                let reported = std::fs::metadata(&opened).map_err(|_| changed())?;
                if reported.dev() == actual.dev() && reported.ino() == actual.ino() {
                    return Ok(());
                }
            }
        }
        #[cfg(windows)]
        if opened != real && windows_other_hard_link_name(file, &opened, named) {
            // GetFinalPathNameByHandle can report another NTFS hard-link name. The bounded
            // protected-identity scan above cleared this handle before either name is accepted.
            return Ok(());
        }
        if opened != real {
            return Err(changed());
        }
        Ok(())
    }

    // Only multiply linked files need the protected-location walk. Missing roots contain no files;
    // an unreadable entry or identity cannot establish that this alias is outside the vault/app state.
    fn check_hard_links(&self, file: &File, deadline: Option<Instant>) -> Result<(), ()> {
        #[cfg(unix)]
        let identity = {
            use std::os::unix::fs::MetadataExt;
            let metadata = file.metadata().map_err(|_| ())?;
            (metadata.dev(), metadata.ino())
        };
        #[cfg(not(unix))]
        let (identity, _) = vw_platform::final_path::identity(file).map_err(|_| ())?;
        self.scan_protected_identity(identity, deadline)
    }

    /// The native hook calls this only for an existing multiply linked file. Rechecking its metadata here keeps the
    /// core authoritative for the candidate as well as the roots; a new file remains on the existing path rule.
    #[cfg(unix)]
    pub(crate) fn check_native_alias(&self, path: &Path) -> Result<(), String> {
        use std::os::unix::fs::MetadataExt;
        let metadata = std::fs::metadata(path).map_err(|_| PROTECTED.to_owned())?;
        if !metadata.is_file() || metadata.nlink() <= 1 {
            return Ok(());
        }
        self.scan_protected_identity((metadata.dev(), metadata.ino()), None)
            .map_err(|()| PROTECTED.into())
    }

    /// Windows obtains the candidate identity and link count from its opened handle. Only multiply linked files
    /// need the bounded protected-root scan; a failed open or identity check is a refusal.
    #[cfg(windows)]
    pub(crate) fn check_native_alias(&self, path: &Path) -> Result<(), String> {
        let metadata = std::fs::metadata(path).map_err(|_| PROTECTED.to_owned())?;
        if !metadata.is_file() {
            return Ok(());
        }
        let file =
            vw_platform::final_path::open_metadata(path).map_err(|_| PROTECTED.to_owned())?;
        let (identity, links) =
            vw_platform::final_path::identity(&file).map_err(|_| PROTECTED.to_owned())?;
        if links <= 1 {
            return Ok(());
        }
        let deadline = self.now() + NATIVE_IDENTITY_BUDGET;
        let protected = self
            .protected_identities(deadline)
            .map_err(|()| PROTECTED.to_owned())?;
        if protected.contains(&identity) {
            Err(PROTECTED.into())
        } else {
            Ok(())
        }
    }

    /// A legacy native grep may search a directory rather than name one file. Inspect every file it could read,
    /// without following links, and share the same deadline with the protected-root walks.
    #[cfg(unix)]
    pub(crate) fn check_native_search(&self, path: &Path) -> Result<(), String> {
        use std::os::unix::fs::MetadataExt;
        let root = path.canonicalize().map_err(|_| PROTECTED.to_owned())?;
        if self.excludes(&root) {
            return Err(PROTECTED.into());
        }
        if !root.is_dir() {
            return self.check_native_alias(&root);
        }
        let deadline = self.now() + IDENTITY_BUDGET;
        let entries = walkdir::WalkDir::new(&root)
            .follow_links(false)
            .follow_root_links(false)
            .max_open(MAX_IDENTITY_ENTRIES.clamp(1, 4096));
        for (visited, entry) in entries.into_iter().enumerate() {
            if visited >= MAX_IDENTITY_ENTRIES || self.now() >= deadline {
                return Err(PROTECTED.into());
            }
            let entry = entry.map_err(|_| PROTECTED.to_owned())?;
            if !entry.file_type().is_file() {
                continue;
            }
            let metadata = entry.metadata().map_err(|_| PROTECTED.to_owned())?;
            if metadata.nlink() <= 1 {
                continue;
            }
            self.scan_protected_identity((metadata.dev(), metadata.ino()), Some(deadline))
                .map_err(|()| PROTECTED.to_owned())?;
        }
        if self.now() >= deadline {
            Err(PROTECTED.into())
        } else {
            Ok(())
        }
    }

    /// A Windows native grep may read every file in a directory. Bound the project walk, then compare its
    /// multiply linked files with one bounded walk of the same protected roots used by opened-file reads.
    #[cfg(windows)]
    pub(crate) fn check_native_search(&self, path: &Path) -> Result<(), String> {
        const MAX_SEARCH_ENTRIES: usize = 100_000;
        const SEARCH_BUDGET: Duration = Duration::from_secs(3);
        let root = path.canonicalize().map_err(|_| PROTECTED.to_owned())?;
        if self.excludes(&root) {
            return Err(PROTECTED.into());
        }
        if !root.is_dir() {
            return self.check_native_alias(&root);
        }
        let deadline = self.now() + SEARCH_BUDGET;
        let mut candidates = Vec::new();
        let entries = walkdir::WalkDir::new(&root)
            .follow_links(false)
            .follow_root_links(false)
            .max_open(4096);
        for (visited, entry) in entries.into_iter().enumerate() {
            if visited >= MAX_SEARCH_ENTRIES || self.now() >= deadline {
                return Err(PROTECTED.into());
            }
            let entry = entry.map_err(|_| PROTECTED.to_owned())?;
            if !entry.file_type().is_file() {
                continue;
            }
            let file = vw_platform::final_path::open_metadata(entry.path())
                .map_err(|_| PROTECTED.to_owned())?;
            let (identity, links) =
                vw_platform::final_path::identity(&file).map_err(|_| PROTECTED.to_owned())?;
            if links > 1 {
                candidates.push(identity);
            }
        }
        if candidates.is_empty() {
            return if self.now() < deadline {
                Ok(())
            } else {
                Err(PROTECTED.into())
            };
        }
        let protected = self
            .protected_identities(deadline)
            .map_err(|()| PROTECTED.to_owned())?;
        if self.now() >= deadline
            || candidates
                .iter()
                .any(|identity| protected.contains(identity))
        {
            Err(PROTECTED.into())
        } else {
            Ok(())
        }
    }

    /// Collect protected identities once for a native file or directory check, using the same roots and entry limit
    /// as `scan_protected_identity`. The shared scan and its 100 ms deadline remain unchanged for app-owned reads.
    #[cfg(windows)]
    fn protected_identities(&self, deadline: Instant) -> Result<Vec<Identity>, ()> {
        let deadline = deadline.min(self.now() + NATIVE_IDENTITY_BUDGET);
        let entry_limit = MAX_IDENTITY_ENTRIES;
        #[cfg(test)]
        let entry_limit = self.entry_limit.unwrap_or(entry_limit);
        let mut visited = 0;
        let mut identities = Vec::new();
        for root in self.data.iter().chain(&self.vaults) {
            if self.now() >= deadline {
                return Err(());
            }
            match std::fs::symlink_metadata(root) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(_) => return Err(()),
                Ok(_) => {}
            }
            let mut entries = walkdir::WalkDir::new(root)
                .follow_links(false)
                .follow_root_links(false)
                .max_open(entry_limit.clamp(1, 4096))
                .into_iter();
            loop {
                if visited >= entry_limit || self.now() >= deadline {
                    return Err(());
                }
                let Some(entry) = entries.next() else { break };
                visited += 1;
                let entry = entry.map_err(|_| ())?;
                if !self.excludes(entry.path()) {
                    if entry.file_type().is_dir() {
                        entries.skip_current_dir();
                    }
                    continue;
                }
                if !entry.file_type().is_file() {
                    continue;
                }
                let file = vw_platform::final_path::open_metadata(entry.path()).map_err(|_| ())?;
                let (identity, links) = vw_platform::final_path::identity(&file).map_err(|_| ())?;
                if links > 1 {
                    identities.push(identity);
                }
            }
        }
        if self.now() >= deadline {
            Err(())
        } else {
            Ok(identities)
        }
    }

    fn scan_protected_identity(
        &self,
        identity: Identity,
        deadline: Option<Instant>,
    ) -> Result<(), ()> {
        let limit = self.now() + IDENTITY_BUDGET;
        let deadline = deadline.map_or(limit, |deadline| deadline.min(limit));
        let entry_limit = MAX_IDENTITY_ENTRIES;
        #[cfg(test)]
        let entry_limit = self.entry_limit.unwrap_or(entry_limit);
        let mut visited = 0;
        for root in self.data.iter().chain(&self.vaults) {
            if self.now() >= deadline {
                return Err(());
            }
            match std::fs::symlink_metadata(root) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(_) => return Err(()),
                Ok(_) => {}
            }
            let mut entries = walkdir::WalkDir::new(root)
                .follow_links(false)
                .follow_root_links(false)
                // Do not buffer an unbounded directory when the walk runs out of open handles.
                .max_open(entry_limit.clamp(1, 4096))
                .into_iter();
            loop {
                if visited >= entry_limit || self.now() >= deadline {
                    return Err(());
                }
                let Some(entry) = entries.next() else { break };
                visited += 1;
                let entry = entry.map_err(|_| ())?;
                if !self.excludes(entry.path()) {
                    if entry.file_type().is_dir() {
                        entries.skip_current_dir();
                    }
                    continue;
                }
                if !entry.file_type().is_file() {
                    continue;
                }
                if self.now() >= deadline {
                    return Err(());
                }
                #[cfg(unix)]
                let protected = {
                    use std::os::unix::fs::MetadataExt;
                    // WalkDir does not follow links. Single-link protected files cannot be aliases
                    // of the multiply-linked candidate; their contents need never be opened.
                    let metadata = entry.metadata().map_err(|_| ())?;
                    if metadata.nlink() <= 1 {
                        continue;
                    }
                    (metadata.dev(), metadata.ino())
                };
                #[cfg(windows)]
                let protected = {
                    // A running app can hold a protected SQLite/log file against a read open.
                    // Its object identity is still available with a metadata-only handle.
                    let file =
                        vw_platform::final_path::open_metadata(entry.path()).map_err(|_| ())?;
                    vw_platform::final_path::identity(&file).map_err(|_| ())?.0
                };
                #[cfg(not(any(unix, windows)))]
                let protected = {
                    let file = File::open(entry.path()).map_err(|_| ())?;
                    vw_platform::final_path::identity(&file).map_err(|_| ())?.0
                };
                if protected == identity {
                    return Err(());
                }
            }
        }
        if self.now() >= deadline {
            Err(())
        } else {
            Ok(())
        }
    }

    #[cfg_attr(
        not(test),
        expect(clippy::unused_self, reason = "tests inject a per-policy clock")
    )]
    fn now(&self) -> Instant {
        #[cfg(test)]
        if let Some(clock) = &self.clock {
            return clock();
        }
        Instant::now()
    }
}

/// A changed Windows final path is harmless only when both ordinary names still identify the
/// opened regular file. The caller must already have checked protected identities and access.
#[cfg(windows)]
pub(crate) fn windows_other_hard_link_name(file: &File, reported: &Path, selected: &Path) -> bool {
    if !file.metadata().is_ok_and(|meta| meta.is_file()) {
        return false;
    }
    let Ok((actual, links)) = vw_platform::final_path::identity(file) else {
        return false;
    };
    if links <= 1 {
        return false;
    }
    for path in [reported, selected] {
        let Ok(real) = path.canonicalize() else {
            return false;
        };
        if vw_platform::plain_path(&real) != vw_platform::plain_path(path) {
            return false;
        }
        let Ok(meta) = std::fs::symlink_metadata(path) else {
            return false;
        };
        if !meta.is_file() || vw_platform::link_entry::is_link(&meta) {
            return false;
        }
        let Ok(at) = vw_platform::final_path::open_metadata(path) else {
            return false;
        };
        if !vw_platform::final_path::identity(&at).is_ok_and(|(identity, _)| identity == actual) {
            return false;
        }
    }
    true
}

#[cfg(test)]
#[path = "read_exclusions_tests.rs"]
pub(crate) mod tests;
