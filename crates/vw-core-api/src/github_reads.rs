//! App-owned gh discovery, credential acquisition and broker lifecycle. Core startup and settings call this.
//! Credentials travel only from app-side gh stdout to broker stdin, never into agent state or files.
//! gh is asked outside the state lock and within [`GH_LIMIT`]: a gh that hangs (a keyring prompt, no network)
//! never holds up a chat start, the git check's `enabled()` or Settings for longer than that.
use crate::{GithubReadsStatus as Status, GithubReadsView as View};
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;
use zeroize::Zeroizing;

/// The longest one `gh auth` call may take; then it is stopped and reported ([`GH_LATE`]).
const GH_LIMIT: Duration = Duration::from_secs(10);
/// The Settings reason when gh did not answer within [`GH_LIMIT`].
const GH_LATE: &str = "gh did not answer within 10 seconds";

/// gh's process while a call waits for it: dropping the call (its command was cancelled, the runtime shut down)
/// before gh answered stops gh's whole group or job, as the timeout does. On Unix `KillOnDrop` alone kills only
/// gh's own pid.
struct GhCall(Box<dyn process_wrap::tokio::ChildWrapper>, bool);

impl Drop for GhCall {
    fn drop(&mut self) {
        if !self.1 {
            let _ = self.0.start_kill();
        }
    }
}

/// Runs `gh <args>` without input. A call still running after [`GH_LIMIT`] is killed and reported as `TimedOut`.
///
/// gh runs as the leader of its own process group (Unix) or in a kill-on-close Job Object (Windows), as the
/// supervisor runs the server, so the timeout, and a call dropped before gh answered, stop every process gh
/// started that stays in that group or job (a credential helper, a pager), not only gh itself. On Unix a process
/// that leaves the group on purpose (`setsid`, a daemon) is out of reach of a group kill: only a cgroup or a
/// subreaper could follow it, and gh's own keyring access starts none (go-keyring: a D-Bus call on Linux, the
/// `security` tool as a plain child on macOS). gh is the user's own program: this is cleanup, not a containment
/// boundary.
async fn gh(program: &Path, args: &[&str]) -> std::io::Result<std::process::Output> {
    use process_wrap::tokio::{CommandWrap, KillOnDrop};
    use tokio::io::AsyncReadExt;
    let mut command = CommandWrap::with_new(program, |cmd| {
        cmd.args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
    });
    #[cfg(unix)]
    command.wrap(process_wrap::tokio::ProcessGroup::leader());
    #[cfg(windows)]
    command.wrap(process_wrap::tokio::JobObject);
    command.wrap(KillOnDrop);
    let mut child = GhCall(command.spawn()?, false);
    let mut stdout = child.0.stdout().take();
    let mut stderr = child.0.stderr().take();
    let read = async {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let read_out = async {
            match stdout.as_mut() {
                Some(pipe) => pipe.read_to_end(&mut out).await.map(|_| ()),
                None => Ok(()),
            }
        };
        let read_err = async {
            match stderr.as_mut() {
                Some(pipe) => pipe.read_to_end(&mut err).await.map(|_| ()),
                None => Ok(()),
            }
        };
        // gh itself (the leader): its exit and the end of both pipes complete the call.
        let (status, (), ()) = tokio::try_join!(child.0.inner_mut().wait(), read_out, read_err)?;
        Ok(std::process::Output {
            status,
            stdout: out,
            stderr: err,
        })
    };
    let answered = tokio::time::timeout(GH_LIMIT, read).await;
    let Ok(output) = answered else {
        // Stop the whole tree (group or job), then reap it briefly so no zombie is left behind.
        let _ = child.0.start_kill();
        let _ = tokio::time::timeout(Duration::from_secs(2), child.0.wait()).await;
        return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, GH_LATE));
    };
    // gh answered (its exit and the end of both pipes): a process it left running on purpose stays, as before.
    child.1 = output.is_ok();
    output
}

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
}

fn discover() -> Option<PathBuf> {
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .map(|p| p.join(if cfg!(windows) { "gh.exe" } else { "gh" }))
        .find(|p| p.is_absolute() && p.is_file())
        .and_then(|p| p.canonicalize().ok())
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
        });
        // A crash must never leave a stale endpoint available to the next run.
        let _ = std::fs::remove_file(reader.endpoint_file());
        reader
    }

    fn endpoint_file(&self) -> PathBuf {
        self.directory.join("endpoint")
    }
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
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

    fn stop(&self, state: &mut State) {
        let _ = std::fs::remove_file(self.endpoint_file());
        if let Some(mut running) = state.running.take() {
            drop(running.child.stdin.take());
            let identity = vw_procmon::identity(running.child.id());
            state.retired.push((running.child, identity));
        }
        state.view.enabled = false;
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

    /// `Err(GH_LATE)` when gh did not answer within [`GH_LIMIT`].
    async fn login(&self) -> Result<(Status, Option<String>), &'static str> {
        let Some(program) = self.real_gh.as_ref().filter(|p| p.is_file()) else {
            return Ok((Status::Missing, None));
        };
        let output = match gh(program, &["auth", "status", "--hostname", "github.com"]).await {
            Ok(output) => output,
            Err(error) if error.kind() == std::io::ErrorKind::TimedOut => return Err(GH_LATE),
            Err(_) => return Ok((Status::SignedOut, None)),
        };
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        if !text.contains("github.com") && output.status.success() {
            return Ok((Status::Enterprise, None));
        }
        if !output.status.success() {
            return Ok((Status::SignedOut, None));
        }
        let login = text
            .lines()
            .find(|line| {
                line.contains("github.com") && (line.contains("account ") || line.contains(" as "))
            })
            .and_then(|line| {
                line.split_once("account ")
                    .or_else(|| line.split_once(" as "))
            })
            .and_then(|(_, rest)| rest.split_whitespace().next())
            .map(str::to_owned);
        if login.is_none() {
            return Ok((Status::SignedOut, None));
        }
        Ok((Status::SignedIn, login))
    }

    async fn token(&self) -> std::io::Result<Zeroizing<String>> {
        let program = self
            .real_gh
            .as_ref()
            .ok_or_else(|| std::io::Error::other("gh is not installed"))?;
        let output = gh(program, &["auth", "token", "--hostname", "github.com"]).await?;
        let bytes = Zeroizing::new(output.stdout);
        if !output.status.success() {
            return Err(std::io::Error::other("gh is not signed in"));
        }
        let token = Zeroizing::new(String::from_utf8_lossy(&bytes).trim().to_owned());
        if token.is_empty() || token.contains(['\n', '\r', '\0']) {
            return Err(std::io::Error::other("gh returned no usable token"));
        }
        Ok(token)
    }

    fn start(&self, token: Zeroizing<String>) -> std::io::Result<Running> {
        let binary = self
            .binary
            .as_ref()
            .ok_or_else(|| std::io::Error::other("the GitHub reader executable is missing"))?;
        if self.shim_prepared.get().is_none() {
            return Err(std::io::Error::other(
                "the GitHub reader shim is unavailable",
            ));
        }
        let gh = self
            .real_gh
            .as_ref()
            .ok_or_else(|| std::io::Error::other("gh is not installed"))?;
        let mut command = Command::new(binary);
        command
            .arg("gh-broker")
            .arg(gh)
            .env_clear()
            .envs(std::env::vars_os().filter(|(k, _)| {
                matches!(
                    k.to_str(),
                    Some(
                        "PATH"
                            | "HOME"
                            | "USERPROFILE"
                            | "SYSTEMROOT"
                            | "WINDIR"
                            | "TEMP"
                            | "TMP"
                            | "TMPDIR"
                            | "LANG"
                            | "LC_ALL"
                            | "HTTP_PROXY"
                            | "HTTPS_PROXY"
                            | "NO_PROXY"
                            | "ALL_PROXY"
                            | "http_proxy"
                            | "https_proxy"
                            | "no_proxy"
                            | "all_proxy"
                            | "SSL_CERT_FILE"
                            | "SSL_CERT_DIR"
                    )
                )
            }))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let mut child = command.spawn()?;
        let result = (|| {
            let input = child
                .stdin
                .as_mut()
                .ok_or_else(|| std::io::Error::other("broker input unavailable"))?;
            writeln!(input, "{}", token.as_str())?;
            let stdout = child
                .stdout
                .take()
                .ok_or_else(|| std::io::Error::other("broker output unavailable"))?;
            let mut endpoint = String::new();
            BufReader::new(stdout).read_line(&mut endpoint)?;
            if endpoint.trim().is_empty() {
                return Err(std::io::Error::other("broker did not start"));
            }
            vw_platform::replace_private(&self.endpoint_file(), endpoint.trim().as_bytes())
                .map_err(std::io::Error::other)
        })();
        if let Err(error) = result {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
        Ok(Running { child, token })
    }

    /// Asks gh (outside the state lock, each call within [`GH_LIMIT`]), then applies its answer under the lock.
    /// `None` only if applying panicked.
    pub(crate) async fn refresh(self: &Arc<Self>, requested: Option<bool>) -> Option<View> {
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

    /// A refresh nobody waits for (a chat start, the app start). It ends within twice [`GH_LIMIT`] plus the
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

    fn maintain(&self, state: &mut State, token: std::io::Result<Zeroizing<String>>) {
        let token = match token {
            Ok(token) => token,
            Err(error) => {
                let was_on = state.running.is_some();
                self.stop(state);
                let late = error.kind() == std::io::ErrorKind::TimedOut;
                state.view.status = if was_on && !late {
                    Status::LoggedOut
                } else {
                    Status::Error
                };
                if late {
                    state.view.reason = Some(GH_LATE.to_owned());
                } else if !was_on {
                    state.view.reason = Some("gh did not provide a usable token".to_owned());
                }
                return;
            }
        };
        let alive = state
            .running
            .as_mut()
            .is_some_and(|r| r.child.try_wait().ok() == Some(None));
        if !alive
            || self.endpoint_file().with_extension("refresh").exists()
            || state
                .running
                .as_ref()
                .is_none_or(|r| r.token.as_str() != token.as_str())
        {
            self.stop(state);
            if let Ok(running) = self.start(token) {
                state.running = Some(running);
            } else {
                state.view.status = Status::Error;
                state.view.reason = Some("the reader process could not be started".to_owned());
            }
        }
        state.view.enabled = state.running.is_some();
        if state.view.enabled {
            state.view.status = Status::On;
        }
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

pub(crate) fn watch(shared: &Arc<crate::core::Shared>) {
    let weak = Arc::downgrade(shared);
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            let Some(shared) = weak.upgrade() else {
                break;
            };
            // Credential checks happen on chat/settings starts or an auth failure, not continuously.
            if shared.github_reads.enabled()
                && shared
                    .github_reads
                    .endpoint_file()
                    .with_extension("refresh")
                    .exists()
            {
                let reader = shared.github_reads.clone();
                let previous = reader.lock().view.clone();
                if let Some(view) = reader.refresh(None).await
                    && view != previous
                {
                    shared.emit(crate::CoreEvent::GithubReads { view });
                }
            }
        }
    });
}

#[cfg(all(test, target_os = "linux"))]
#[path = "github_reads_tests.rs"]
mod tests;
