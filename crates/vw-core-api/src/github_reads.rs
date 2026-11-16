//! App-owned gh discovery, credential acquisition and broker lifecycle. Core startup and settings call this.
//! Depends on: `process_wrap`, `serde_json` and the core lifecycle. Called by: core startup, session launch, Settings
//! and, for the shim's auth-failure marker, agent commands ([`refresh_if_marked`]); nothing here runs on a timer.
//! Credentials travel only from app-side gh stdout to broker stdin, never into agent state or files.
//! gh is asked outside the state lock and within [`GH_LIMIT`]: a gh that hangs (a keyring prompt, no network)
//! never holds up a chat start, the git check's `enabled()` or Settings for longer than that.
use crate::{GithubReadsStatus as Status, GithubReadsView as View};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use vw_platform::sync::LockRecover;
use zeroize::Zeroizing;

#[path = "github_reads_broker.rs"]
mod broker;
#[path = "github_reads_gh.rs"]
mod gh_calls;
use gh_calls::discover;

/// The longest one `gh auth` call may take; then it is stopped and reported ([`GH_LATE`]).
const GH_LIMIT: Duration = Duration::from_secs(10);
/// The Settings reason when gh did not answer within [`GH_LIMIT`].
const GH_LATE: &str = "gh did not answer within 10 seconds";

/// What gh answered, asked before the state lock is taken.
struct Probe {
    status: Status,
    login: Option<String>,
    /// Why `status` is `Error` (gh did not answer in time).
    reason: Option<String>,
    /// The token: asked only when reads are wanted and gh is signed in.
    token: Option<std::io::Result<Zeroizing<String>>>,
}

struct Running {
    child: Child,
    token: Zeroizing<String>,
}
struct State {
    view: View,
    running: Option<Running>,
    retired: Vec<(Child, Option<vw_procmon::ProcessRef>)>,
    /// Set by `shutdown`: a gh answer that arrives after quit starts nothing.
    closed: bool,
}

pub(crate) struct Reader {
    state: Mutex<State>,
    binary: Option<PathBuf>,
    real_gh: Option<PathBuf>,
    directory: PathBuf,
    settings: Option<PathBuf>,
    shim_prepared: std::sync::OnceLock<()>,
    /// A marker refresh is running ([`refresh_if_marked`]); a second agent command does not start another.
    marked_refresh: std::sync::atomic::AtomicBool,
}

impl Reader {
    pub(crate) fn new(config: &crate::CoreConfig, home: &Path) -> Arc<Self> {
        let directory = config.agent_tools.as_ref().map_or_else(
            || home.join("github-reader"),
            |t| t.data_dir.join("github-reader"),
        );
        let settings = config
            .secret_scan_data
            .as_ref()
            .map(|p| p.join("settings.json"));
        let enabled = settings
            .as_ref()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .and_then(|s| s["githubReads"].as_bool())
            .unwrap_or(false);
        let reader = Arc::new(Self {
            state: Mutex::new(State {
                view: View {
                    enabled,
                    status: Status::Missing,
                    login: None,
                    reason: None,
                },
                running: None,
                retired: Vec::new(),
                closed: false,
            }),
            binary: config.agent_tools.as_ref().map(|t| t.binary.clone()),
            real_gh: config.gh_program.clone().or_else(discover),
            directory,
            settings,
            shim_prepared: std::sync::OnceLock::new(),
            marked_refresh: std::sync::atomic::AtomicBool::new(false),
        });
        // A crash must never leave a stale endpoint available to the next run.
        let _ = std::fs::remove_file(reader.endpoint_file());
        reader
    }

    fn endpoint_file(&self) -> PathBuf {
        self.directory.join("endpoint")
    }
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock_or_recover()
    }

    fn save(&self, enabled: bool) -> std::io::Result<()> {
        if let Some(path) = &self.settings {
            crate::write_app_settings(
                path,
                serde_json::Map::from_iter([(
                    "githubReads".to_owned(),
                    serde_json::json!(enabled),
                )]),
            )?;
        }
        Ok(())
    }

    /// Add only non-secret routing information after the ordinary credential filter.
    pub(crate) fn environment(&self, env: &mut Vec<(OsString, OsString)>) -> std::io::Result<()> {
        let Some(binary) = &self.binary else {
            return Ok(());
        };
        std::fs::create_dir_all(&self.directory)?;
        let shim_dir = self.directory.join("bin");
        std::fs::create_dir_all(&shim_dir)?;
        std::fs::create_dir_all(shim_dir.join("local-config"))?;
        let shim = shim_dir.join(if cfg!(windows) { "gh.exe" } else { "gh" });
        if self.shim_prepared.get().is_none() {
            let replacement = shim.with_extension("new");
            std::fs::copy(binary, &replacement)?;
            std::fs::rename(replacement, &shim)?;
            let _ = self.shim_prepared.set(());
        }
        let path = env
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("PATH"))
            .map(|(_, v)| v.clone())
            .unwrap_or_default();
        let mut paths = vec![shim_dir];
        paths.extend(std::env::split_paths(&path));
        let path = std::env::join_paths(paths).map_err(std::io::Error::other)?;
        env.retain(|(k, _)| {
            !k.eq_ignore_ascii_case("PATH")
                && !k.eq_ignore_ascii_case("VW_GH_BROKER_ENDPOINT")
                && !k.eq_ignore_ascii_case("VW_GH_LOCAL_EXECUTABLE")
        });
        env.push(("PATH".into(), path));
        env.push((
            "VW_GH_BROKER_ENDPOINT".into(),
            format!("file:{}", self.endpoint_file().display()).into(),
        ));
        if let Some(gh) = &self.real_gh {
            env.push(("VW_GH_LOCAL_EXECUTABLE".into(), gh.as_os_str().to_owned()));
        }
        Ok(())
    }

    /// Asks gh (outside the state lock, each call within [`GH_LIMIT`]), then applies its answer under the lock.
    /// `None` only if applying panicked.
    pub(crate) async fn refresh(self: &Arc<Self>, requested: Option<bool>) -> Option<View> {
        tracing::debug!(reader = %self.endpoint_file().display(), "checking GitHub read credentials");
        let wanted = requested.unwrap_or_else(|| self.enabled());
        let probe = match self.login().await {
            Ok((status, login)) => {
                let token = if wanted && status == Status::SignedIn {
                    Some(self.token().await)
                } else {
                    None
                };
                Probe {
                    status,
                    login,
                    reason: None,
                    token,
                }
            }
            Err(late) => Probe {
                status: Status::Error,
                login: None,
                reason: Some(late.to_owned()),
                token: None,
            },
        };
        let reader = self.clone();
        // Starting the broker blocks on its first output line: kept off the async threads.
        tokio::task::spawn_blocking(move || reader.apply(requested, probe))
            .await
            .ok()
    }

    /// A refresh nobody waits for (a chat start, the app start). It ends within three times [`GH_LIMIT`] plus the
    /// broker start, and after `shutdown` it changes nothing.
    pub(crate) fn refresh_later(self: &Arc<Self>) {
        let reader = self.clone();
        tokio::spawn(async move {
            let _ = reader.refresh(None).await;
        });
    }

    fn apply(&self, requested: Option<bool>, probe: Probe) -> View {
        let mut state = self.lock();
        let enabled = requested.unwrap_or(state.view.enabled);
        // Reads were switched on while gh was asked without a token: the refresh that switched them on applies.
        if state.closed || (enabled && probe.status == Status::SignedIn && probe.token.is_none()) {
            return state.view.clone();
        }
        if requested == Some(false) {
            self.stop(&mut state);
        }
        let Probe {
            status,
            login,
            reason,
            token,
        } = probe;
        let was_on = state.running.is_some();
        let previous = state.view.clone();
        state
            .retired
            .retain_mut(|(child, _)| child.try_wait().ok() == Some(None));
        state.view.login = login;
        state.view.reason = reason;
        state.view.status = status.clone();
        if !enabled || status != Status::SignedIn {
            self.stop(&mut state);
            if status == Status::SignedOut && (was_on || previous.status == Status::LoggedOut) {
                state.view.status = Status::LoggedOut;
            } else if requested.is_none()
                && previous.status == Status::Error
                && status == Status::SignedIn
            {
                state.view.status = Status::Error;
                state.view.reason = previous.reason;
            }
        } else if let Some(token) = token {
            self.maintain(&mut state, token);
        }
        if (requested.is_some() || previous.enabled != state.view.enabled)
            && self.save(state.view.enabled).is_err()
        {
            self.stop(&mut state);
            state.view.status = Status::Error;
            state.view.reason = Some("the setting could not be saved".to_owned());
        }
        let _ = std::fs::remove_file(self.endpoint_file().with_extension("refresh"));
        state.view.clone()
    }

    pub(crate) fn enabled(&self) -> bool {
        self.lock().view.enabled
    }
    pub(crate) fn shutdown(&self) {
        let mut state = self.lock();
        state.closed = true;
        self.stop(&mut state);
        for (mut child, identity) in state.retired.drain(..) {
            if let Some(target) = identity {
                let _ = vw_procmon::stop(
                    target,
                    &vw_procmon::Authority::Ledger,
                    &vw_procmon::Inputs::default(),
                    &vw_procmon::StopConfig::default(),
                );
            }
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl crate::Core {
    pub(crate) async fn github_reads(
        &self,
        request: crate::GithubReadsRequest,
    ) -> Result<crate::CommandResult, crate::CoreError> {
        let reader = self.shared.github_reads.clone();
        let requested = match request {
            crate::GithubReadsRequest::Read {} => None,
            crate::GithubReadsRequest::Write { enabled } => Some(enabled),
        };
        let view = reader.refresh(requested).await.ok_or_else(|| {
            crate::CoreError::new(crate::ErrorCode::Internal, "GitHub reader unavailable")
        })?;
        self.shared
            .emit(crate::CoreEvent::GithubReads { view: view.clone() });
        Ok(crate::CommandResult::GithubReads { view })
    }
}

pub(crate) fn prepare(
    reader: &Arc<Reader>,
    supervisor: &mut vw_supervisor::SupervisorConfig,
    config: &mut crate::CoreConfig,
) {
    let mut env = supervisor
        .parent_env
        .clone()
        .unwrap_or_else(|| std::env::vars_os().collect());
    // Explicit launch PATH wins over the process's PATH, as in the supervisor's environment builder.
    for (key, value) in &supervisor.extra_env {
        env.retain(|(name, _)| !name.eq_ignore_ascii_case(key));
        env.push((key.into(), value.clone()));
    }
    if reader.environment(&mut env).is_err() {
        let mut state = reader.lock();
        reader.stop(&mut state);
        state.view.status = Status::Error;
        state.view.reason = Some("the reader executable could not be prepared".to_owned());
        return;
    }
    for (key, value) in env.iter().filter(|(k, _)| {
        matches!(
            k.to_str(),
            Some("PATH" | "VW_GH_BROKER_ENDPOINT" | "VW_GH_LOCAL_EXECUTABLE")
        )
    }) {
        supervisor
            .extra_env
            .retain(|(k, _)| !key.eq_ignore_ascii_case(k));
        supervisor
            .extra_env
            .push((key.to_string_lossy().into_owned(), value.clone()));
        config
            .terminals
            .extra_env
            .push((key.clone(), value.clone()));
    }
    if let Some(tools) = &mut config.agent_tools {
        let _ = reader.environment(&mut tools.process_env);
    }
    // The app start does not wait on gh either: the reader turns on when gh answers.
    if reader.enabled() {
        reader.refresh_later();
    }
}

/// How long an agent command waits for a marker refresh before it runs anyway. Under the hook's 2 s
/// `shell_command` budget (`vw-agent-tools` `decide_safety`), so the report itself is never cut off; a slower refresh
/// finishes on its own and the command after it uses the result.
const MARKED_WAIT: Duration = Duration::from_millis(1500);

/// The per-read shim (`vw-gh-broker` `entry.rs`) leaves `<endpoint>.refresh` when gh refused the broker's token.
/// It is looked at only here, right before an agent command that can run `gh` (the pre-tool hook's shell-command
/// report, and the core's own `process_start`/`process_restart`/`secret_run`), never on a timer. With the marker
/// there and reads on, the reader refreshes once (`apply` removes the marker), Settings hears of a changed view, and
/// the command continues. Chat start and Settings keep their own refreshes.
pub(crate) async fn refresh_if_marked(shared: &Arc<crate::core::Shared>) {
    use std::sync::atomic::Ordering;
    /// Clears the flag however the refresh ends.
    struct Running(Arc<Reader>);
    impl Drop for Running {
        fn drop(&mut self) {
            self.0.marked_refresh.store(false, Ordering::SeqCst);
        }
    }
    let reader = &shared.github_reads;
    if !reader.enabled()
        || !reader.endpoint_file().with_extension("refresh").exists()
        || reader.marked_refresh.swap(true, Ordering::SeqCst)
    {
        return;
    }
    let running = Running(reader.clone());
    let weak = Arc::downgrade(shared);
    // Detached: a hook that stops waiting must not cancel gh halfway (dropping the call kills gh).
    let task = tokio::spawn(async move {
        let reader = running.0.clone();
        let previous = reader.lock().view.clone();
        let view = reader.refresh(None).await;
        drop(running);
        if let (Some(view), Some(shared)) = (view, weak.upgrade())
            && view != previous
        {
            shared.emit(crate::CoreEvent::GithubReads { view });
        }
    });
    let _ = tokio::time::timeout(MARKED_WAIT, task).await;
}
