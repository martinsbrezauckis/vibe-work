//! Prepares the app-owned startup file for agent Bash commands. Depends on std and config; called by the
//! launcher and environment builder. Only non-interactive child shells source this file.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

const STARTUP: &str =
    "# Agent command shells must report failures anywhere in a pipeline.\nset -o pipefail\n";

pub(crate) fn path(home: &Path) -> PathBuf {
    home.join("agent-shell.bash")
}

pub(crate) fn prepare(home: &Path) -> Result<(), crate::config::SupervisorError> {
    write(home).map_err(|source| crate::config::SupervisorError::ShellStartup {
        path: path(home),
        source,
    })
}

fn write(home: &Path) -> std::io::Result<()> {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    std::fs::create_dir_all(home)?;
    let temp = home.join(format!(
        ".agent-shell-{}-{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)?;
    let result = (|| {
        file.write_all(STARTUP.as_bytes())?;
        drop(file);
        // Replace the directory entry without following a planted startup-file symlink.
        std::fs::rename(&temp, path(home))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}
