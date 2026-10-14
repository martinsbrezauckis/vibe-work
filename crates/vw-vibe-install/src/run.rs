//! The install task: the steps in order, their progress on the view channel, cleanup on failure,
//! and the hand-off to the `on_ready` hook that starts Vibe.
//!
//! Steps (the offer lists exactly these commands):
//! 1. Download uv: Astral's archive, SHA-256 checked before unpacking; only `uv(.exe)` is taken out.
//! 2. Install Python: `uv python install <py> --no-bin` (+ `--no-registry` on Windows: no PEP 514 registry entry)
//!    into the install folder (`UV_PYTHON_INSTALL_DIR`); nothing on PATH.
//! 3. Create the environment: `uv venv --python <py> --no-project --no-config <venv>` (no Python download here).
//! 4. Install Vibe: `uv pip install --require-hashes --only-binary:all: --compile-bytecode --link-mode copy
//!    --index-url https://pypi.org/simple --no-config -r requirements.txt` (every package and hash pinned; no
//!    source builds; bytecode compiled once so the first start is fast; copies, so the uv cache can go).
//! 5. Check: the `mistral_vibe-<version>.dist-info` and the server executable exist; sizes; `installed.json`.
//! 6. Start Vibe: the `on_ready` hook.
//!
//! Every uv step: `UV_NO_CONFIG=1` (a user's uv.toml / index settings cannot redirect it), `UV_CACHE_DIR` in the
//! install folder, `UV_SYSTEM_CERTS=1` (the OS trust store, so corporate CAs work), `UV_NO_PROGRESS=1`, the proxy
//! and certificate variables of the allowlist ([`crate::steps::environment`]).
//!
//! A Vibe **update** ([`Slot::Update`]) runs the same steps into its own version folder next to the
//! running install, with one difference in step 1: the hash-locked requirements file comes from the release channel
//! and must have the SHA-256 the signed manifest states before anything else is downloaded. Its progress goes to the
//! view's `update` part (the running Vibe and the setup phase stay as they are); it ends with `active.json` written
//! (the next start uses it) instead of a start.
//!
//! Depends on: [`crate::fetch`], [`crate::steps`], [`crate::layout`], [`crate::manager`]. Called by: the manager.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::watch;
use tracing::{info, warn};

use crate::fetch::{self, FetchError};
use crate::layout::{Installed, Layout, has_dist_info, tree_bytes};
use crate::manager::{Inner, Limits};
use crate::pin::{Sources, UvAsset};
use crate::steps::{self, Step, StepError, Tail};
use crate::view::{FailKind, InstallPhase, InstallSource, VibeUpdatePhase};

const TOTAL: u32 = 6;
/// An update has no start step (it starts with Vibe Work's next start).
const UPDATE_TOTAL: u32 = 5;

/// Which part of the view an install reports to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Slot {
    /// The install at setup (the main phase).
    Setup,
    /// A Vibe update next to the running install.
    Update,
}

/// One install to run.
pub(crate) struct Job {
    pub(crate) sources: Sources,
    pub(crate) layout: Layout,
    pub(crate) slot: Slot,
    /// The requirements file to download and check (updates); `None`: `sources.requirements` (the embedded pin).
    pub(crate) requirements: Option<UvAsset>,
}

pub(crate) fn installing(
    step: &str,
    index: u32,
    elapsed_ms: u64,
    output_tail: String,
) -> InstallPhase {
    InstallPhase::Installing {
        step: step.to_owned(),
        index,
        total: TOTAL,
        elapsed_ms,
        output_tail,
    }
}

pub(crate) fn update_installing(
    step: &str,
    index: u32,
    elapsed_ms: u64,
    output_tail: String,
) -> VibeUpdatePhase {
    VibeUpdatePhase::Installing {
        step: step.to_owned(),
        index,
        total: UPDATE_TOTAL,
        elapsed_ms,
        output_tail,
    }
}

/// The uv commands of steps 2-4, as they run.
pub(crate) fn steps(sources: &Sources, layout: &Layout, limits: &Limits) -> Vec<Step> {
    let uv = layout.uv_dir().join(fetch::uv_name());
    let os = |s: &str| OsString::from(s);
    let mut python_args = vec![
        os("python"),
        os("install"),
        os(&sources.python),
        os("--no-bin"),
    ];
    if cfg!(windows) {
        python_args.push(os("--no-registry"));
    }
    vec![
        Step {
            name: format!("Install Python {}", sources.python),
            program: uv.clone(),
            args: python_args,
            limit: limits.python,
        },
        Step {
            name: "Create the environment".to_owned(),
            program: uv.clone(),
            args: vec![
                os("venv"),
                os("--python"),
                os(&sources.python),
                os("--no-project"),
                os("--no-config"),
                layout.venv().into_os_string(),
            ],
            limit: limits.venv,
        },
        Step {
            name: format!("Install Vibe {}", sources.vibe_version),
            program: uv,
            args: vec![
                os("pip"),
                os("install"),
                os("--python"),
                layout.venv_python().into_os_string(),
                os("--require-hashes"),
                os("--only-binary"),
                os(":all:"),
                os("--compile-bytecode"),
                os("--link-mode"),
                os("copy"),
                os("--index-url"),
                os(&sources.index_url),
                os("--no-config"),
                os("-r"),
                layout
                    .install_dir()
                    .join("requirements.txt")
                    .into_os_string(),
            ],
            limit: limits.packages,
        },
    ]
}

fn uv_env(inner: &Inner, layout: &Layout, step_index: usize) -> Vec<(OsString, OsString)> {
    let dir = layout.install_dir();
    let mut own: Vec<(String, OsString)> = vec![
        ("UV_NO_CONFIG".into(), "1".into()),
        ("UV_NO_PROGRESS".into(), "1".into()),
        ("UV_SYSTEM_CERTS".into(), "1".into()),
        ("UV_CACHE_DIR".into(), dir.join("cache").into_os_string()),
        (
            "UV_PYTHON_INSTALL_DIR".into(),
            dir.join("python").into_os_string(),
        ),
        (
            "UV_PYTHON_BIN_DIR".into(),
            dir.join("python-bin").into_os_string(),
        ),
        ("UV_PYTHON_PREFERENCE".into(), "only-managed".into()),
        // Only step 2 may download a Python; later steps use the one installed.
        (
            "UV_PYTHON_DOWNLOADS".into(),
            if step_index == 0 {
                "automatic"
            } else {
                "never"
            }
            .into(),
        ),
    ];
    own.extend(inner.config.extra_env.iter().cloned());
    steps::environment(inner.config.parent_env.iter().cloned(), &own)
}

struct Run<'a> {
    inner: &'a Arc<Inner>,
    started: Instant,
    slot: Slot,
}

impl Run<'_> {
    fn progress(&self, step: &str, index: u32, tail: String) {
        let mut state = self.inner.lock();
        let elapsed = millis(self.started);
        match self.slot {
            Slot::Setup if matches!(state.phase, InstallPhase::Installing { .. }) => {
                state.phase = installing(step, index, elapsed, tail);
            }
            Slot::Update if matches!(state.update, VibeUpdatePhase::Installing { .. }) => {
                state.update = update_installing(step, index, elapsed, tail);
            }
            _ => return,
        }
        self.inner.publish(&state);
    }
}

fn millis(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

struct Failure {
    step: String,
    kind: FailKind,
    error: String,
    tail: String,
}

/// The install, from the download to the start. Publishes progress; ends in `installed` or `failed`.
pub(crate) async fn install(inner: Arc<Inner>, job: Job, cancel: watch::Receiver<bool>) {
    let started = Instant::now();
    let run = Run {
        inner: &inner,
        started,
        slot: job.slot,
    };
    if job.slot == Slot::Update {
        let outcome = install_steps(&run, &job, cancel).await;
        update_outcome(&inner, &job.layout, outcome);
        return;
    }
    let layout = &job.layout;
    match install_steps(&run, &job, cancel).await {
        Ok(record) => {
            info!(
                ms = record.install_ms,
                bytes = record.disk_bytes,
                "Vibe installed"
            );
            layout.clear_choice();
            let installed = InstallPhase::Installed {
                version: record.version,
                server: record.server.display().to_string(),
                source: InstallSource::Private,
                disk_bytes: Some(record.disk_bytes),
                install_ms: Some(record.install_ms),
                warning: None,
            };
            start(&inner, record.server, installed, "Start Vibe", TOTAL).await;
        }
        Err(failure) => {
            warn!(step = %failure.step, kind = ?failure.kind, error = %failure.error, "Vibe install failed");
            if let Err(error) = layout.remove_unfinished() {
                warn!(%error, "the unfinished install folder could not be removed");
            }
            let mut state = inner.lock();
            state.cancel = None;
            state.phase = InstallPhase::Failed {
                step: failure.step,
                kind: failure.kind,
                error: failure.error,
                output_tail: failure.tail,
            };
            inner.publish(&state);
        }
    }
}

/// The end of an update install: `active.json` names the new version (the next start uses it) and the update
/// part says `installed`; or `failed` with the unfinished folder removed. The running Vibe is never touched.
fn update_outcome(inner: &Inner, layout: &Layout, outcome: Result<Installed, Failure>) {
    let outcome = outcome.and_then(|record| {
        crate::layout::save_active(&layout.root, &record.version)
            .map(|()| record)
            .map_err(|e| disk("Check the install", &e))
    });
    let phase = match outcome {
        Ok(record) => {
            info!(version = %record.version, ms = record.install_ms, "Vibe update installed; used from the next start");
            VibeUpdatePhase::Installed {
                version: record.version,
            }
        }
        Err(failure) => {
            warn!(step = %failure.step, kind = ?failure.kind, error = %failure.error, "Vibe update failed");
            if let Err(error) = layout.remove_unfinished() {
                warn!(%error, "the unfinished update folder could not be removed");
            }
            VibeUpdatePhase::Failed {
                step: failure.step,
                kind: failure.kind,
                error: failure.error,
                output_tail: failure.tail,
            }
        }
    };
    let mut state = inner.lock();
    state.cancel = None;
    state.update = phase;
    inner.publish(&state);
}

/// Runs the `on_ready` hook and publishes `installed` (or `failed` with kind `start`).
pub(crate) async fn start(
    inner: &Arc<Inner>,
    server: PathBuf,
    installed: InstallPhase,
    step: &str,
    index: u32,
) {
    {
        let mut state = inner.lock();
        state.phase = installing(step, index, 0, String::new());
        inner.publish(&state);
    }
    let outcome = match inner.on_ready() {
        Some(hook) => hook(server).await,
        None => Ok(()),
    };
    let mut state = inner.lock();
    state.cancel = None;
    state.phase = match outcome {
        Ok(()) => installed,
        Err(error) => InstallPhase::Failed {
            step: step.to_owned(),
            kind: FailKind::Start,
            error,
            output_tail: String::new(),
        },
    };
    inner.publish(&state);
}

fn fail(step: &str, kind: FailKind, error: String, tail: String) -> Failure {
    Failure {
        step: step.to_owned(),
        kind,
        error,
        tail,
    }
}

fn disk(step: &str, error: &std::io::Error) -> Failure {
    fail(step, FailKind::Disk, error.to_string(), String::new())
}

/// Steps 1-5: download, uv steps, check. Ends with `installed.json` written (the commit point).
async fn install_steps(
    run: &Run<'_>,
    job: &Job,
    cancel: watch::Receiver<bool>,
) -> Result<Installed, Failure> {
    let (sources, layout) = (&job.sources, &job.layout);
    download_uv(run, job, cancel.clone()).await?;
    run_uv_steps(run, sources, layout, cancel).await?;
    finish(run, sources, layout)
}

/// Kind of a failed download.
fn fetch_kind(error: &FetchError) -> FailKind {
    match error {
        FetchError::Network { .. } | FetchError::Status { .. } => FailKind::Network,
        FetchError::Checksum { .. } | FetchError::TooLarge(_) | FetchError::Archive(_) => {
            FailKind::Checksum
        }
        FetchError::Timeout(_) => FailKind::Timeout,
        FetchError::Disk { .. } => FailKind::Disk,
    }
}

/// Step 1: a fresh install folder, the requirements file (embedded, or for an update downloaded and checked against
/// the signed SHA-256 first), uv downloaded (checksum first) and unpacked.
async fn download_uv(
    run: &Run<'_>,
    job: &Job,
    mut cancel: watch::Receiver<bool>,
) -> Result<(), Failure> {
    let (sources, layout) = (&job.sources, &job.layout);
    let dir = layout.install_dir();
    let first = match job.slot {
        Slot::Setup => "Download uv",
        Slot::Update => "Download the requirements and uv",
    };
    layout.remove_unfinished().map_err(|e| disk(first, &e))?;
    for sub in ["dl", "uv"] {
        std::fs::create_dir_all(dir.join(sub)).map_err(|e| disk(first, &e))?;
    }
    let requirements = dir.join("requirements.txt");
    match &job.requirements {
        None => {
            std::fs::write(&requirements, &sources.requirements).map_err(|e| disk(first, &e))?;
        }
        Some(asset) => {
            run.progress(first, 1, format!("{} from {}", asset.sha256, asset.url));
            let fetched = tokio::select! {
                result = fetch::download(asset, &requirements, run.inner.config.limits.download, |_| {}) => result,
                _ = cancel.wait_for(|c| *c) => {
                    return Err(fail(first, FailKind::Cancelled, "cancelled".into(), String::new()));
                }
            };
            fetched.map_err(|e| {
                fail(
                    first,
                    fetch_kind(&e),
                    format!("the requirements file: {e}"),
                    String::new(),
                )
            })?;
        }
    }
    let archive = dir.join("dl").join(format!("uv.{}", sources.uv.archive));
    let mut next_report = 0_u64;
    let download = fetch::download(
        &sources.uv,
        &archive,
        run.inner.config.limits.download,
        |received| {
            if received >= next_report {
                next_report = received + 1024 * 1024;
                let tail = format!(
                    "{} of {} bytes from {}",
                    received, sources.uv.bytes, sources.uv.url
                );
                run.progress(first, 1, tail);
            }
        },
    );
    let fetched = tokio::select! {
        result = download => result,
        _ = cancel.wait_for(|c| *c) => {
            return Err(fail(first, FailKind::Cancelled, "cancelled".into(), String::new()));
        }
    };
    fetched.map_err(|error| fail(first, fetch_kind(&error), error.to_string(), String::new()))?;
    fetch::unpack_uv(&archive, &sources.uv.archive, &layout.uv_dir())
        .map_err(|e| fail(first, FailKind::Checksum, e.to_string(), String::new()))?;
    Ok(())
}

/// Steps 2-4: the uv commands, with their progress and output tail.
async fn run_uv_steps(
    run: &Run<'_>,
    sources: &Sources,
    layout: &Layout,
    cancel: watch::Receiver<bool>,
) -> Result<(), Failure> {
    let dir = layout.install_dir();
    for (index, step) in steps(sources, layout, &run.inner.config.limits)
        .iter()
        .enumerate()
    {
        let number = u32::try_from(index).unwrap_or(0) + 2;
        run.progress(&step.name, number, step.display(&dir));
        let env = uv_env(run.inner, layout, index);
        let mut tail = Tail::default();
        let mut lines_since = 0_u32;
        let mut shown = Tail::default();
        let result = steps::run(step, &env, &dir, cancel.clone(), &mut tail, |line| {
            shown.push_line(line);
            lines_since += 1;
            if lines_since >= 5 {
                lines_since = 0;
                run.progress(&step.name, number, shown.text());
            }
        })
        .await;
        result.map_err(|error| {
            let kind = match error {
                StepError::Timeout(_) => FailKind::Timeout,
                StepError::Cancelled => FailKind::Cancelled,
                StepError::Failed(_) | StepError::Spawn(_) => network_or_step(&tail.text()),
            };
            fail(
                &step.name,
                kind,
                format!("{} {error}", step.name),
                tail.text(),
            )
        })?;
    }
    Ok(())
}

/// Step 5: Vibe and its server are there; the download and cache go; `installed.json` is written last.
fn finish(run: &Run<'_>, sources: &Sources, layout: &Layout) -> Result<Installed, Failure> {
    let dir = layout.install_dir();
    let check = "Check the install";
    run.progress(check, 5, String::new());
    if !has_dist_info(&layout.venv(), "mistral_vibe", &sources.vibe_version)
        || !layout.server().is_file()
    {
        return Err(fail(
            check,
            FailKind::Step,
            format!(
                "mistral-vibe {} or its app server is missing after the install",
                sources.vibe_version
            ),
            String::new(),
        ));
    }
    for sub in ["dl", "cache"] {
        let _ = std::fs::remove_dir_all(dir.join(sub));
    }
    let record = Installed {
        version: sources.vibe_version.clone(),
        server: layout.server(),
        uv: sources.uv_version.clone(),
        requirements_sha256: sources.requirements_sha256.clone(),
        disk_bytes: tree_bytes(&dir),
        install_ms: millis(run.started),
        finished_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs()),
    };
    let text = serde_json::to_string_pretty(&record)
        .map_err(|e| fail(check, FailKind::Disk, e.to_string(), String::new()))?;
    std::fs::write(layout.marker(), text).map_err(|e| disk(check, &e))?;
    Ok(record)
}

/// uv's own words for a connection problem make the failure a network one (clear message, "check your
/// connection or proxy"), everything else a step failure.
fn network_or_step(tail: &str) -> FailKind {
    let lower = tail.to_ascii_lowercase();
    let network = [
        "failed to fetch",
        "dns error",
        "connection refused",
        "error sending request",
        "tcp connect error",
        "network is unreachable",
        "timed out",
        "proxy",
        "certificate",
    ];
    if network.iter().any(|n| lower.contains(n)) {
        FailKind::Network
    } else {
        FailKind::Step
    }
}
