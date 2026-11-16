//! Runtime refresh: Vibe Work's own install of the pinned Vibe, made from older hash-locked requirements (an earlier
//! app version's pin), gets the current requirements without a version change. That is how a security fix in a
//! dependency reaches people who already have Vibe installed.
//!
//! - When: [`VibeInstaller::refresh_runtime`], called by the core once Vibe has started (in the background; no
//!   prompt: the app update that brought the new pin is the person's choice, as for
//!   [`VibeInstaller::upgrade_previous_private`]). Only when the running install is the private one of the pinned
//!   version and its `installed.json` `requirementsSha256` (a SHA-256, as the installer always records) differs
//!   from the embedded pin. The user's own install and a newer Vibe update are never touched.
//! - How: the setup's own steps ([`crate::run`], [`crate::run::Slot::Refresh`]) into `<version>/venv-<hash>` beside
//!   the running venv, `--require-hashes` as always. The new server must then answer the version probe
//!   ([`crate::existing::probe`], a throwaway Vibe home) as the pinned version before `installed.json` is switched
//!   to it (temp file + rename). The running Vibe keeps running; the next start uses the new venv and removes the
//!   old one ([`crate::layout::Layout::remove_other_venvs`]).
//! - Failure (offline, a failed step, a probe that does not answer): the new venv is removed and the old runtime is
//!   used as before; the next start tries again. The person's Vibe home, config and sessions live outside the
//!   install folder and are never read or written.
//!
//! Depends on: [`crate::run`], [`crate::layout`], [`crate::existing`]. Called by: vw-core-api (`core_start`), tests.

use std::path::PathBuf;
use std::time::Instant;

use tokio::sync::watch;
use tracing::{info, warn};

use crate::layout::Installed;
use crate::manager::{Inner, VibeInstaller};
use crate::run::{Failure, Job, Run, Slot, fail};
use crate::view::{FailKind, InstallPhase, InstallSource};

/// What [`VibeInstaller::refresh_runtime`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeRefresh {
    /// Nothing to do: the private install already has the pinned requirements, or the running Vibe is not Vibe Work's
    /// own install of the pinned version (none yet, the user's own, a newer update, a failed start).
    NotNeeded,
    /// Another refresh is running.
    Running,
    /// The new environment is installed and answered as the pinned Vibe; the next start uses `server`.
    Refreshed {
        /// The new environment's `vibe-app-server`.
        server: PathBuf,
    },
    /// The refresh failed; the old runtime is unchanged and the next start tries again.
    Failed {
        /// The step that failed.
        step: String,
        /// What kind of failure.
        kind: FailKind,
        /// Why.
        error: String,
    },
}

/// The check step's name in a refresh failure.
const CHECK: &str = "Check the install";

impl VibeInstaller {
    /// Rebuilds the running private install's environment from the pinned requirements when it was made from other
    /// ones (see the module docs). Runs to the end; the core calls it on a background task.
    pub async fn refresh_runtime(&self) -> RuntimeRefresh {
        let Some(job) = self.refresh_job() else {
            return RuntimeRefresh::NotNeeded;
        };
        {
            let mut state = self.inner.lock();
            if state.refreshing {
                return RuntimeRefresh::Running;
            }
            state.refreshing = true;
        }
        info!(version = %job.sources.vibe_version, "the Vibe runtime is refreshed from newer pinned requirements");
        let run = Run {
            inner: &self.inner,
            started: Instant::now(),
            slot: Slot::Refresh,
        };
        // Nothing cancels a refresh: it ends with the app or at a step's time limit.
        let (_never, cancel) = watch::channel(false);
        let outcome = crate::run::install_steps(&run, &job, cancel).await;
        self.inner.lock().refreshing = false;
        match outcome {
            Ok(record) => {
                info!(
                    ms = record.install_ms,
                    "Vibe runtime refreshed; used from the next start"
                );
                RuntimeRefresh::Refreshed {
                    server: record.server,
                }
            }
            Err(failure) => {
                warn!(step = %failure.step, kind = ?failure.kind, error = %failure.error,
                    "Vibe runtime refresh failed; the current runtime stays in use");
                remove_attempt(&job);
                RuntimeRefresh::Failed {
                    step: failure.step,
                    kind: failure.kind,
                    error: failure.error,
                }
            }
        }
    }

    /// The refresh to run, if the running install needs one.
    fn refresh_job(&self) -> Option<Job> {
        let sources = self.inner.config.sources.clone().ok()?;
        let layout = self.inner.layout.clone()?;
        let running_private = matches!(
            &self.inner.lock().phase,
            InstallPhase::Installed { version, source: InstallSource::Private, .. } if *version == self.inner.pinned
        );
        if !running_private || layout.choice().is_some() {
            return None;
        }
        let record = layout.installed()?;
        // The installer always records the SHA-256 it installed; a record without one is not its own (a fixture).
        let recorded = &record.requirements_sha256;
        let is_sha256 = recorded.len() == 64 && recorded.bytes().all(|b| b.is_ascii_hexdigit());
        if !is_sha256 || *recorded == sources.requirements_sha256 {
            return None;
        }
        let target = layout.refresh(&sources.requirements_sha256)?;
        // A server that runs from the target folder (an earlier refresh under the same name) is never rebuilt.
        if record.server.starts_with(target.venv()) {
            return None;
        }
        Some(Job {
            sources,
            layout: target,
            slot: Slot::Refresh,
            requirements: None,
        })
    }
}

/// The new server answers the version probe as the pinned version (a throwaway Vibe home under `probe/`).
pub(crate) async fn answers_as(inner: &Inner, record: &Installed) -> Result<(), Failure> {
    let Some(layout) = inner.layout.as_ref() else {
        return Err(fail(
            CHECK,
            FailKind::Step,
            "no install folder".into(),
            String::new(),
        ));
    };
    let home = layout.probe_dir().join("refresh");
    let found = crate::existing::probe(&record.server, &home, &inner.config.parent_env).await;
    if let Err(error) = vw_platform::remove_owned_tree(&layout.root, &home) {
        warn!(%error, "the refresh probe home could not be removed");
    }
    match found {
        Some(version) if version == record.version => Ok(()),
        found => Err(fail(
            CHECK,
            FailKind::Step,
            format!(
                "the new Vibe environment did not answer as Vibe {} ({})",
                record.version,
                found.map_or_else(|| "no answer".to_owned(), |v| format!("it said {v}"))
            ),
            String::new(),
        )),
    }
}

/// Removes what a failed refresh left: its venv, its requirements file, the download and cache folders. Never the
/// running venv (the job's venv is a different folder) and never `installed.json`.
fn remove_attempt(job: &Job) {
    let layout = &job.layout;
    let dir = layout.install_dir();
    for path in [layout.venv(), dir.join("dl"), dir.join("cache")] {
        if path.exists()
            && let Err(error) = std::fs::remove_dir_all(&path)
        {
            warn!(%error, folder = %path.display(), "a failed refresh's folder was not removed (the next start removes it)");
        }
    }
    let _ = std::fs::remove_file(layout.requirements());
}
