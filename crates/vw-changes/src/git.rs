//! The one way this crate runs git: hardened config, an allowlisted environment, our own index file, a
//! timeout and an output cap.
//!
//! Adapted from ShellX `src-tauri/src/session_git.rs:22-29,832-856` @ e38c2b5c (`GIT_HARDENING_CONFIG`,
//! `GIT_TERMINAL_PROMPT=0`) and hardened: the Changes tab runs git on its own
//! in folders the user merely opened, and a repository's `.git/config` + `.gitattributes` can make git run
//! programs (filters, diff drivers, external diff, fsmonitor, hooks, pager). This module pins every such
//! key; the callers (`objects`, `worktree`, `review`) additionally never let git read or write the working
//! tree, which is where filters run.
//! Depends on: std, `hooks` (the private empty hooks folder; no git call without it). Called by: `repo`, `objects`,
//! `worktree`, `baseline`, `diff`, `review`, `plain`, and other crates that run git (`vw-worktrees`: [`Call`],
//! [`Index`] and [`Output`] are public so every git call in the app goes through this one hardened runner).
//! Invariants:
//! - Every call names an index file. Repository calls pass ours (`GIT_INDEX_FILE` in our app data), so even
//!   git's opportunistic index refresh writes our file; only `plain::init_repository` passes `Index::User`,
//!   for the one action the user confirmed ("git init + first commit").
//! - The environment is cleared and rebuilt from an allowlist (`ENV_ALLOW`): no inherited `GIT_*`
//!   (`GIT_DIR`, `GIT_EXTERNAL_DIFF`, `GIT_CONFIG_PARAMETERS`,...) reaches git; `GIT_CONFIG_NOSYSTEM=1`,
//!   `GIT_OPTIONAL_LOCKS=0`, `GIT_TERMINAL_PROMPT=0`, `LC_ALL=C` (stable messages for error matching).
//! - Config that runs programs or changes output we parse is pinned per call (`HARDENING` + hooks folder).
//! - A call never blocks forever: it is killed after its timeout, and its stdout is capped.
//! - Two wider [`Profile`]s for the user's own Commit/merge (`Worktree`: the system config, where
//!   Git for Windows keeps `core.autocrlf`) and Push (`Publish`: also the user's credential helpers, ssh and the
//!   ssh agent). Both keep every other pin; callers check the repository's own config first (vw-git `programs`).
//! - The commit the user presses runs the repository's commit hooks, like
//!   `git commit` in a terminal ([`Call::with_repo_hooks`], a per-project switch, on by default). Only that call:
//!   every implicit call keeps the private empty hooks folder.
//! - Git never gets a Windows verbatim path (`\\?\C:\…`): the working folder, the index file, the hooks
//!   folder and every argument that is (or ends in, after `=`) such a path go in their plain form (`for_git`).
//!   Git for Windows cannot use them (hooks are not found, `--file=` and pathspec files are not read).

use std::borrow::Cow;
use std::ffi::{OsStr, OsString};
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::error::{ChangesError, clip};

/// `-c key=value` pairs on every call (command-line config wins over every config file). Programs git
/// could run: fsmonitor, pager, external diff, ssh, credential helpers and askpass, any transport (we never
/// fetch or push: `protocol.allow=never`); what keeps our temporary index ours (`splitIndex` would write
/// `sharedindex.*` into `.git`); no housekeeping in the user's repository (`gc.auto`, `maintenance.auto`);
/// stable output for the parser; paths past 260 characters on Windows (`core.longpaths`, below). The hooks folder
/// is added per call (`hooks::current`).
pub(crate) const HARDENING: &[(&str, &str)] = &[
    ("core.fsmonitor", "false"),
    ("core.untrackedCache", "false"),
    ("core.pager", "cat"),
    ("diff.external", ""),
    ("core.sshCommand", ""),
    ("core.askPass", ""),
    ("credential.helper", ""),
    ("protocol.allow", "never"),
    ("protocol.ext.allow", "never"),
    ("core.splitIndex", "false"),
    ("gc.auto", "0"),
    ("maintenance.auto", "false"),
    ("color.ui", "false"),
    ("core.quotePath", "false"),
    ("core.safecrlf", "false"),
    ("diff.noprefix", "false"),
    ("diff.mnemonicPrefix", "false"),
    ("diff.relative", "false"),
    ("submodule.recurse", "false"),
    ("commit.gpgSign", "false"),
    // Git for Windows stops at 260 characters unless this is on, so a deep app data folder (the baseline's copy of
    // the project's ignore rules lives in `<data>/sessions/<id>/ignore/...`, one level per project folder) or a deep
    // project could not be read. Given per call, so nothing is written to any config. Only Git for Windows reads the
    // key; git on Linux and macOS never parses it (it accepts even a value that is not a boolean), so it is set on
    // every platform.
    ("core.longpaths", "true"),
];

/// Keys of [`HARDENING`] the `Publish` profile leaves to the user's own system and global config (the
/// repository's own values for them are refused before a push by the caller).
const PUBLISH_OPEN: &[&str] = &[
    "core.sshCommand",
    "core.askPass",
    "credential.helper",
    "protocol.allow",
];

/// Transports a push may use: https, ssh, git and local/file (`ext::`, plain http and remote helpers stay off;
/// vw-git also refuses such addresses before it calls git).
const PUBLISH_PROTOCOLS: &[(&str, &str)] = &[
    ("protocol.allow", "never"),
    ("protocol.ext.allow", "never"),
    ("protocol.https.allow", "always"),
    ("protocol.ssh.allow", "always"),
    ("protocol.git.allow", "always"),
    ("protocol.file.allow", "always"),
    ("push.gpgSign", "false"),
];

/// Extra variables a push may inherit: the ssh agent, the desktop session a credential helper may show a
/// prompt in (Linux secret service, Git Credential Manager), proxies and certificates, the `gh` credential
/// helper's own settings, and the Windows profile variables credential managers read.
const PUBLISH_ENV: &[&str] = &[
    "SSH_AUTH_SOCK",
    "SSH_AGENT_PID",
    "SSH_ASKPASS",
    "SSH_ASKPASS_REQUIRE",
    "DISPLAY",
    "WAYLAND_DISPLAY",
    "XDG_RUNTIME_DIR",
    "DBUS_SESSION_BUS_ADDRESS",
    "HTTPS_PROXY",
    "HTTP_PROXY",
    "ALL_PROXY",
    "NO_PROXY",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "GH_TOKEN",
    "GITHUB_TOKEN",
    "GH_HOST",
    "GH_CONFIG_DIR",
    "GH_ENTERPRISE_TOKEN",
    "USERNAME",
    "USERDOMAIN",
    "SYSTEMDRIVE",
    "PROGRAMFILES",
    "PROGRAMFILES(X86)",
    "COMMONPROGRAMFILES",
];

/// How much of the user's own git setup a call may use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    /// Review and listing calls: no system config, no network, no credentials.
    Review,
    /// The user's own Commit and "Bring into main": git reads and writes the working tree with the
    /// system and global config (line-ending conversion), still no network and no credentials.
    Worktree,
    /// The user's own Push: like `Worktree`, plus the user's credential helpers and ssh from the
    /// system and global config, the ssh agent and proxies; transports limited to https/ssh/git/file.
    Publish,
}

/// Environment variables git may inherit (everything else is dropped). Enough to find git and the user's
/// own global config (`HOME`, `XDG_CONFIG_HOME`; on Windows the profile variables) and a temp folder.
const ENV_ALLOW: &[&str] = &[
    "PATH",
    "HOME",
    "XDG_CONFIG_HOME",
    "USER",
    "LOGNAME",
    "TMPDIR",
    "TMP",
    "TEMP",
    "USERPROFILE",
    "HOMEDRIVE",
    "HOMEPATH",
    "APPDATA",
    "LOCALAPPDATA",
    "PROGRAMDATA",
    "SYSTEMROOT",
    "WINDIR",
    "COMSPEC",
    "PATHEXT",
];

/// Largest stdout a call may print (a whole-repository patch of a big session stays far below it).
pub(crate) const OUTPUT_CAP: usize = 64 << 20;
/// Stderr kept for error messages.
const STDERR_CAP: usize = 16 << 10;
/// Default time limit of one call.
pub(crate) const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// Which index file git uses.
#[derive(Debug, Clone, Copy)]
pub enum Index<'a> {
    /// Ours (in the app data folder).
    Ours(&'a Path),
    /// The repository's own index. Only for `plain::init_repository` (a user-confirmed `git init` + commit),
    /// for calls that read no index at all (`rev-parse`, `--version`), and for read-only index reads of a
    /// linked worktree (`diff-index --cached`; `GIT_OPTIONAL_LOCKS=0` keeps git from writing it).
    User,
}

/// What a finished call returned.
#[derive(Debug)]
pub struct Output {
    /// Exit code (`None` when killed by a signal).
    pub code: Option<i32>,
    /// Standard output.
    pub stdout: Vec<u8>,
    /// Standard error, clipped.
    pub stderr: String,
}

impl Output {
    /// True when git exited with 0.
    #[must_use]
    pub fn ok(&self) -> bool {
        self.code == Some(0)
    }
}

/// One git call.
pub struct Call<'a> {
    cwd: &'a Path,
    index: Index<'a>,
    args: Vec<OsString>,
    stdin: Option<&'a [u8]>,
    timeout: Duration,
    cap: usize,
    profile: Profile,
    repo_hooks: bool,
    cancel: Option<&'a AtomicBool>,
    prefix: bool,
}

impl<'a> Call<'a> {
    /// A call of `git <args>` in `cwd` with `index`.
    #[must_use]
    pub fn new(cwd: &'a Path, index: Index<'a>) -> Self {
        Self {
            cwd,
            index,
            args: Vec::new(),
            stdin: None,
            timeout: DEFAULT_TIMEOUT,
            cap: OUTPUT_CAP,
            profile: Profile::Review,
            repo_hooks: false,
            cancel: None,
            prefix: false,
        }
    }

    /// Lets git run the repository's own hooks (`core.hooksPath` or `.git/hooks`) and gives them the user's
    /// environment (like a terminal: without loader hijacks, `GIT_*`, `VW_*`, the Mistral key and `PORT`).
    /// only the user's own Commit with the "Run this repository's commit hooks" switch on.
    #[must_use]
    pub fn with_repo_hooks(mut self) -> Self {
        self.repo_hooks = true;
        self
    }

    /// Uses a wider [`Profile`] (the user's Commit and Push only).
    #[must_use]
    pub fn profile(mut self, profile: Profile) -> Self {
        self.profile = profile;
        self
    }

    /// Adds arguments.
    #[must_use]
    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.args
            .extend(args.into_iter().map(|a| a.as_ref().to_os_string()));
        self
    }

    /// Feeds `bytes` to git's stdin.
    #[must_use]
    pub fn stdin(mut self, bytes: &'a [u8]) -> Self {
        self.stdin = Some(bytes);
        self
    }

    /// Overrides the stdout cap (reading one big blob back for a revert).
    #[must_use]
    pub fn max_output(mut self, cap: usize) -> Self {
        self.cap = cap;
        self
    }

    /// Overrides the time limit.
    #[must_use]
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Stops a read when the caller replaces its search. No process outlives cancellation.
    #[must_use]
    pub fn cancel_when(mut self, token: &'a AtomicBool) -> Self {
        self.cancel = Some(token);
        self
    }

    /// Returns the bounded stdout prefix when a search fills its result budget.
    #[must_use]
    pub fn take_prefix(mut self) -> Self {
        self.prefix = true;
        self
    }

    fn name(&self) -> String {
        self.args
            .first()
            .map(|a| a.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    /// Runs the call and returns stdout when git exited with 0.
    ///
    /// # Errors
    /// `Git` with clipped stderr on a non-zero exit; see [`Call::output`] for the rest.
    pub fn run(self) -> Result<Vec<u8>, ChangesError> {
        let name = self.name();
        let out = self.output()?;
        if out.ok() {
            Ok(out.stdout)
        } else {
            Err(ChangesError::Git {
                command: name,
                message: if out.stderr.is_empty() {
                    format!("exit {:?}", out.code)
                } else {
                    clip(&out.stderr, 400)
                },
            })
        }
    }

    /// Runs the call and returns its output whatever the exit code.
    ///
    /// # Errors
    /// `GitMissing` when git cannot be started, `Timeout`, `TooMuchOutput`, `Io` on pipe failures.
    pub fn output(self) -> Result<Output, ChangesError> {
        let name = self.name();
        // Fail closed: no git without our private, empty hooks folder (`hooks`).
        let hooks = crate::hooks::current()?;
        let mut command = build(
            self.cwd,
            self.index,
            &self.args,
            self.stdin.is_some(),
            &hooks,
            self.profile,
            self.repo_hooks,
        );
        let mut child = command.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                ChangesError::GitMissing(e.to_string())
            } else {
                ChangesError::io("start git in", self.cwd, e)
            }
        })?;
        let input = self.stdin.map(<[u8]>::to_vec);
        let writer = child.stdin.take().map(|mut pipe| {
            std::thread::spawn(move || {
                if let Some(bytes) = input {
                    // A git that exits early closes the pipe: the write error is not the failure to report.
                    let _ = pipe.write_all(&bytes);
                }
            })
        });
        let overflow = Arc::new(AtomicBool::new(false));
        let stdout = child
            .stdout
            .take()
            .map(|pipe| spawn_reader(pipe, self.cap, Some(overflow.clone())));
        let stderr = child
            .stderr
            .take()
            .map(|pipe| spawn_reader(pipe, STDERR_CAP, None));
        let deadline = Instant::now() + self.timeout;
        let mut nap = Duration::from_micros(200);
        let status = loop {
            if let Some(status) = child
                .try_wait()
                .map_err(|e| ChangesError::io("wait for git in", self.cwd, e))?
            {
                break status;
            }
            if overflow.load(Ordering::Relaxed)
                || Instant::now() >= deadline
                || self.cancel.is_some_and(|c| c.load(Ordering::Relaxed))
            {
                let _ = child.kill();
                let _ = child.wait();
                if overflow.load(Ordering::Relaxed) && self.prefix {
                    if let Some(writer) = writer {
                        let _ = writer.join();
                    }
                    let stdout = stdout
                        .map(|h| h.join().unwrap_or_default())
                        .unwrap_or_default();
                    if let Some(stderr) = stderr {
                        let _ = stderr.join();
                    }
                    return Ok(Output {
                        code: Some(0),
                        stdout,
                        stderr: String::new(),
                    });
                }
                if overflow.load(Ordering::Relaxed) {
                    return Err(ChangesError::TooMuchOutput {
                        command: name,
                        limit: self.cap,
                    });
                }
                return Err(ChangesError::Timeout {
                    command: name,
                    seconds: self.timeout.as_secs(),
                });
            }
            std::thread::sleep(nap);
            nap = (nap * 2).min(Duration::from_millis(8));
        };
        if let Some(writer) = writer {
            let _ = writer.join();
        }
        let stdout = stdout
            .map(|h| h.join().unwrap_or_default())
            .unwrap_or_default();
        let stderr = stderr
            .map(|h| h.join().unwrap_or_default())
            .unwrap_or_default();
        if overflow.load(Ordering::Relaxed) && !self.prefix {
            return Err(ChangesError::TooMuchOutput {
                command: name,
                limit: self.cap,
            });
        }
        Ok(Output {
            code: status.code(),
            stdout,
            stderr: String::from_utf8_lossy(&stderr).into_owned(),
        })
    }
}

/// Builds the process: hardened `-c` pairs, clean environment, our index.
fn build(
    cwd: &Path,
    index: Index<'_>,
    args: &[OsString],
    with_stdin: bool,
    hooks_dir: &Path,
    profile: Profile,
    repo_hooks: bool,
) -> Command {
    let mut command = Command::new("git");
    // Every path git gets is in its plain form (see `for_git`); the working folder too, although
    // Windows already hands the child the plain form of a verbatim working folder (measured on Windows 11).
    command.current_dir(vw_platform::plain_path(cwd));
    for (key, value) in HARDENING {
        if profile == Profile::Publish && PUBLISH_OPEN.contains(key) {
            continue;
        }
        command.arg("-c").arg(format!("{key}={value}"));
    }
    if profile == Profile::Publish {
        for (key, value) in PUBLISH_PROTOCOLS {
            command.arg("-c").arg(format!("{key}={value}"));
        }
    }
    if !repo_hooks {
        let mut hooks = OsString::from("core.hooksPath=");
        hooks.push(vw_platform::plain_path(hooks_dir));
        command.arg("-c").arg(hooks);
    }
    command.args(args.iter().map(|a| for_git(a)));
    command.env_clear();
    for (key, value) in std::env::vars_os() {
        let name = key.to_string_lossy().to_ascii_uppercase();
        if ENV_ALLOW.contains(&name.as_str())
            || (profile == Profile::Publish && PUBLISH_ENV.contains(&name.as_str()))
            || (repo_hooks && hook_env(&name))
        {
            command.env(key, value);
        }
    }
    if profile == Profile::Review {
        command.env("GIT_CONFIG_NOSYSTEM", "1");
    }
    if profile == Profile::Publish {
        // git's own transport allowlist: applies to every address git resolves (after insteadOf too) and wins
        // over any `protocol.*.allow` config (let git enforce it, not our parser).
        command.env("GIT_ALLOW_PROTOCOL", "https:ssh:git:file");
    }
    command
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C")
        .env("LANGUAGE", "C");
    if let Index::Ours(path) = index {
        command.env("GIT_INDEX_FILE", vw_platform::plain_path(path));
    }
    command
        .stdin(if with_stdin {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    no_window(&mut command);
    command
}

/// One argument as git gets it: a Windows verbatim path (`\\?\C:\x`, what `canonicalize` answers), alone
/// or as the value of an option or config pair (`--file=\\?\C:\x`, `core.hooksPath=\\?\C:\x`), in its plain form.
/// Git for Windows (2.55, probed on Windows 11) cannot use verbatim paths: it appends
/// `/name` to a hooks folder (a verbatim path takes `/` literally: "cannot spawn …\hooks/pre-commit"), reads
/// `--file=` and `--pathspec-from-file=` as `//?/C:/…` ("No such file"), and fails on a verbatim
/// `GIT_INDEX_FILE` ("Invalid argument"). A verbatim working folder (`-C` or the process's) works.
/// Only what `dunce` can simplify without changing the file named is rewritten (drive paths under the legacy
/// length, no reserved names); every other argument, and every argument on other systems, passes unchanged.
fn for_git(arg: &OsStr) -> Cow<'_, OsStr> {
    if !cfg!(windows) {
        return Cow::Borrowed(arg);
    }
    let Some(text) = arg.to_str() else {
        return Cow::Borrowed(arg);
    };
    let (head, path) = if text.starts_with(r"\\?\") {
        ("", text)
    } else if let Some(at) = text.find(r"=\\?\") {
        text.split_at(at + 1)
    } else {
        return Cow::Borrowed(arg);
    };
    let plain = vw_platform::plain_path(Path::new(path));
    if plain.as_os_str() == path {
        return Cow::Borrowed(arg);
    }
    let mut out = OsString::from(head);
    out.push(plain);
    Cow::Owned(out)
}

/// The environment the repository's own hooks get (the user's, as a terminal gets it in vw-terminal `env.rs`):
/// everything except loader and runtime hijack variables, `GIT_*`, Vibe Work's `VW_*`, the Mistral key and
/// `PORT`.
fn hook_env(upper: &str) -> bool {
    const DROP_PREFIX: &[&str] = &["GIT_", "VW_", "LD_", "DYLD_", "PYTHON"];
    const DROP: &[&str] = &[
        "MISTRAL_API_KEY",
        "BASH_ENV",
        "ENV",
        "NODE_OPTIONS",
        "PERL5OPT",
        "RUBYOPT",
        "PORT",
    ];
    !DROP_PREFIX.iter().any(|p| upper.starts_with(p)) && !DROP.contains(&upper)
}

#[cfg(windows)]
fn no_window(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    /// `CREATE_NO_WINDOW`: no console window flashes for each git call.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn no_window(_command: &mut Command) {}

/// Reads a pipe to the end, keeping at most `cap` bytes. With `overflow`, going over the cap sets the flag
/// (the caller kills git) and the rest is drained and dropped so git never blocks on a full pipe.
fn spawn_reader(
    mut pipe: impl Read + Send + 'static,
    cap: usize,
    overflow: Option<Arc<AtomicBool>>,
) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut kept = Vec::new();
        let mut buf = vec![0_u8; 64 * 1024];
        loop {
            match pipe.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let room = cap.saturating_sub(kept.len());
                    let take = n.min(room);
                    kept.extend_from_slice(&buf[..take]);
                    if take < n
                        && let Some(flag) = &overflow
                    {
                        flag.store(true, Ordering::Relaxed);
                    }
                }
            }
        }
        kept
    })
}

/// The installed git's version as `(major, minor)`, `None` when git cannot be started or prints nonsense.
#[must_use]
pub fn version(cwd: &Path) -> Option<(u32, u32)> {
    let out = Call::new(cwd, Index::User)
        .args(["--version"])
        .timeout(Duration::from_secs(10))
        .run()
        .ok()?;
    let text = String::from_utf8_lossy(&out);
    let numbers = text.split_whitespace().nth(2)?;
    let mut parts = numbers.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    Some((major, minor))
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::for_git;
    use std::ffi::OsStr;

    /// verbatim drive paths lose their prefix alone and after `=`; other arguments stay as they are.
    #[test]
    fn verbatim_paths_reach_git_in_their_plain_form() {
        let cases = [
            (r"\\?\C:\work\repo\.git\hooks", r"C:\work\repo\.git\hooks"),
            (
                r"--file=\\?\C:\data\tmp\commit-message",
                r"--file=C:\data\tmp\commit-message",
            ),
            (
                r"core.hooksPath=\\?\C:\data\hooks",
                r"core.hooksPath=C:\data\hooks",
            ),
            (
                r"--pathspec-from-file=\\?\C:\d\paths",
                r"--pathspec-from-file=C:\d\paths",
            ),
            ("status", "status"),
            (r"C:\plain\path", r"C:\plain\path"),
            // Network and reserved forms keep the prefix (their plain form would name something else).
            (r"\\?\UNC\server\share\x", r"\\?\UNC\server\share\x"),
            (r"\\?\C:\work\CON", r"\\?\C:\work\CON"),
        ];
        for (input, want) in cases {
            assert_eq!(for_git(OsStr::new(input)), OsStr::new(want), "{input}");
        }
    }
}
