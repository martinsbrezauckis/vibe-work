//! Finds the `vibe-app-server` executable.
//!
//! Depends on: std only. Called by: the app (to show "Vibe found at ..." or a setup hint) and the
//! supervisor config. The server version is not read here: `vibe-app-server` 2.26.0 has no
//! `--version` flag (it exits with "unrecognized arguments"), so the version comes from the
//! `initialize` answer (`serverInfo.version`, see `handshake`).
//! Invariants: order is explicit setting → `PATH` → known install locations; an explicit setting
//! that is unusable is an error, never silently replaced by another binary; only absolute paths
//! are ever considered (a relative `PATH` entry or setting would resolve against whatever the
//! current directory is, possibly an agent-controlled project folder, and the spawn resolves a
//! relative program against the *child's* cwd on some platforms: a differential an attacker can
//! plant into); the result is canonical (absolute, symlinks resolved) and is exactly what the
//! supervisor spawns; on Windows only a
//! real `.exe` is accepted (no `.cmd`/`.bat`: `cmd.exe` re-parses arguments, the batbadbut (CVE-2024-24576) class
//! ShellX had to fence off in `provider_adapters.rs:3018-3036 @ e38c2b5c`).
//!
//! Adapted from ShellX `src-tauri/src/provider_runtime.rs:15-152 @ e38c2b5c` (user bin dirs,
//! case-insensitive dedup on Windows). Changed: ShellX searched user dirs before `PATH` and
//! accepted any `PATHEXT` script; unrelated tool dirs (other agent CLIs, .bun, npm) dropped.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// File name of the server executable on this OS.
pub const BINARY_NAME: &str = if cfg!(windows) {
    "vibe-app-server.exe"
} else {
    "vibe-app-server"
};

/// Where a discovered binary came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FoundVia {
    /// The explicit path setting.
    Explicit,
    /// A directory on `PATH`.
    Path,
    /// A known install location (`~/.local/bin` and friends).
    KnownLocation,
}

/// A discovered server binary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discovered {
    /// Absolute or as-given path of the executable.
    pub path: PathBuf,
    /// How it was found.
    pub via: FoundVia,
}

/// Discovery failure.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DiscoveryError {
    /// The explicit setting is a relative path (only absolute settings are accepted).
    #[error("the configured vibe-app-server path must be absolute: {0}")]
    ExplicitNotAbsolute(PathBuf),
    /// The explicit setting points at something that is not a usable executable.
    #[error("the configured vibe-app-server path is not an executable file: {0}")]
    ExplicitNotUsable(PathBuf),
    /// Nothing found; `searched` lists every candidate tried (for the setup hint).
    #[error("vibe-app-server not found (searched {} locations); install Mistral Vibe or set its path", .searched.len())]
    NotFound {
        /// Every candidate path tried, in order.
        searched: Vec<PathBuf>,
    },
}

/// Inputs for discovery; injectable so tests never depend on the machine's real `PATH`.
#[derive(Debug, Clone, Default)]
pub struct Discovery {
    /// Explicit path setting (wins when set).
    pub explicit: Option<PathBuf>,
    /// Value of `PATH`.
    pub path_var: Option<OsString>,
    /// The user's home (`HOME`, or `USERPROFILE` on Windows).
    pub home: Option<PathBuf>,
}

impl Discovery {
    /// Discovery from this process's environment plus an optional explicit setting.
    #[must_use]
    pub fn from_env(explicit: Option<PathBuf>) -> Self {
        let home_var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
        Self {
            explicit,
            path_var: std::env::var_os("PATH"),
            home: std::env::var_os(home_var).map(PathBuf::from),
        }
    }

    /// Every candidate in search order (explicit excluded).
    #[must_use]
    pub fn candidates(&self) -> Vec<PathBuf> {
        self.candidates_split().0
    }

    /// Candidates plus how many of them came from `PATH` (they come first).
    fn candidates_split(&self) -> (Vec<PathBuf>, usize) {
        let mut out: Vec<PathBuf> = Vec::new();
        let push = |dir: PathBuf, out: &mut Vec<PathBuf>| {
            let candidate = dir.join(BINARY_NAME);
            if !out.iter().any(|seen| same_path(seen, &candidate)) {
                out.push(candidate);
            }
        };
        if let Some(path_var) = &self.path_var {
            for dir in std::env::split_paths(path_var) {
                // Empty and relative entries ("", ".", "bin", "./node_modules/.bin") resolve
                // against the current directory: never search them.
                if dir.is_absolute() {
                    push(dir, &mut out);
                }
            }
        }
        let from_path = out.len();
        for dir in known_dirs(self.home.as_deref()) {
            push(dir, &mut out);
        }
        (out, from_path)
    }

    /// Finds the server binary.
    ///
    /// # Errors
    /// [`DiscoveryError::ExplicitNotUsable`] when the explicit path is set but unusable;
    /// [`DiscoveryError::NotFound`] when no candidate exists.
    pub fn discover(&self) -> Result<Discovered, DiscoveryError> {
        if let Some(explicit) = &self.explicit {
            if !explicit.is_absolute() {
                return Err(DiscoveryError::ExplicitNotAbsolute(explicit.clone()));
            }
            return match canonical_executable(explicit) {
                Some(path) => Ok(Discovered {
                    path,
                    via: FoundVia::Explicit,
                }),
                None => Err(DiscoveryError::ExplicitNotUsable(explicit.clone())),
            };
        }
        let (candidates, from_path) = self.candidates_split();
        for (index, candidate) in candidates.iter().enumerate() {
            if let Some(path) = canonical_executable(candidate) {
                let via = if index < from_path {
                    FoundVia::Path
                } else {
                    FoundVia::KnownLocation
                };
                return Ok(Discovered { path, via });
            }
        }
        Err(DiscoveryError::NotFound {
            searched: candidates,
        })
    }
}

/// Known install locations after `PATH`. `uv tool install mistral-vibe` puts the scripts in
/// `~/.local/bin` (`%USERPROFILE%\.local\bin` on Windows); a desktop app started from the dock or
/// Start menu often lacks that on `PATH` (the reason for ShellX `f0493cfe`).
fn known_dirs(home: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = home.filter(|h| h.is_absolute()) {
        dirs.push(home.join(".local").join("bin"));
    }
    if cfg!(not(windows)) {
        dirs.push(PathBuf::from("/opt/homebrew/bin"));
        dirs.push(PathBuf::from("/usr/local/bin"));
    }
    dirs
}

fn same_path(a: &Path, b: &Path) -> bool {
    if cfg!(windows) {
        a.to_string_lossy()
            .eq_ignore_ascii_case(&b.to_string_lossy())
    } else {
        a == b
    }
}

/// The canonical form of `path` if it is an executable file: absolute, symlinks resolved, and
/// on Windows without the `\\?\` prefix for plain drive paths (launchers such as the uv
/// trampoline locate their interpreter from their own path and need the normal form).
#[must_use]
pub fn canonical_executable(path: &Path) -> Option<PathBuf> {
    let canonical = std::fs::canonicalize(path).ok()?;
    let canonical = strip_verbatim(canonical);
    is_executable(&canonical).then_some(canonical)
}

#[cfg(windows)]
fn strip_verbatim(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => PathBuf::from(rest),
        _ => path,
    }
}

#[cfg(not(windows))]
fn strip_verbatim(path: PathBuf) -> PathBuf {
    path
}

/// A regular file we may execute: on Unix any execute bit set, on Windows a `.exe` file.
#[must_use]
pub fn is_executable(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        path.extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
    }
}
