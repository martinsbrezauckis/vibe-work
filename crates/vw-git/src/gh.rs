//! GitHub's own CLI (`gh`) for creating a pull request and reading its card,
//! when the user has it installed and signed in. Vibe Work never stores a forge token: gh keeps its own.
//!
//! Every call runs in a neutral folder of ours (never the repository, so gh reads no repository config and runs
//! no git there), names the repository with `--repo HOST/OWNER/REPO`, has prompts, colours, pager and the
//! update notice off, a time limit and an output cap. gh gets the user's environment (it needs the keyring
//! and its own config) without `GIT_*` variables.
//!
//! Depends on: `serde_json`, `pr`, `forge`, `error`. Called by: vw-core-api.
//! Invariant: the only writing call is [`create`], made after the user's confirmation in the core.

use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::error::{GitError, clip};
use crate::forge::Forge;
use crate::names;
use crate::pr::{self, PrCard, ReviewComment};

#[path = "gh_head.rs"]
mod head;
pub use head::head_for_branch;

/// The JSON fields the card reads.
pub const VIEW_FIELDS: &str = "number,url,title,state,isDraft,reviewDecision,mergeable,mergeStateStatus,statusCheckRollup,headRefName,baseRefName";

const OUTPUT_CAP: usize = 8 << 20;
const READ_TIMEOUT: Duration = Duration::from_secs(45);
const CREATE_TIMEOUT: Duration = Duration::from_secs(120);

/// Where gh is and the neutral folder its calls run in.
#[derive(Debug, Clone)]
pub struct Gh {
    program: PathBuf,
    workdir: PathBuf,
}

/// The installed gh: `explicit` when given (tests), else [`locate`]. `workdir` must be a private, existing
/// folder of ours.
#[must_use]
pub fn find(explicit: Option<&Path>, workdir: &Path) -> Option<Gh> {
    let program = match explicit {
        Some(p) => p.is_file().then(|| p.to_path_buf())?,
        None => locate()?,
    };
    Some(Gh {
        program,
        workdir: workdir.to_path_buf(),
    })
}

/// The one gh lookup every part of the app uses (pull request cards, GitHub reads): the first `gh` on `PATH` or
/// in the usual install folders of this system. A desktop app started from Finder, the Dock or a Start menu
/// often gets a short `PATH` without the package manager's folder.
#[must_use]
pub fn locate() -> Option<PathBuf> {
    locate_with(InstallLayout::current(), |key| std::env::var_os(key))
}

/// [`locate`] for a given layout and environment (tests). Only absolute paths count: a relative `PATH` entry
/// would name a program in whatever folder the app runs in.
#[must_use]
pub fn locate_with(
    layout: InstallLayout,
    env: impl Fn(&str) -> Option<OsString>,
) -> Option<PathBuf> {
    candidates_for(layout, env)
        .into_iter()
        .find(|p| p.is_absolute() && p.is_file())
}

/// Whose install folders [`candidates_for`] lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallLayout {
    /// The GitHub CLI installer and winget (Program Files, the user's Programs, winget's links), Scoop.
    Windows,
    /// Homebrew (Apple silicon and Intel), Mac ports (`/opt/local`), the user's `~/.local/bin`.
    MacOs,
    /// The distribution, Snap, Homebrew on Linux, the user's `~/.local/bin`.
    Linux,
}

impl InstallLayout {
    /// This system's layout.
    #[must_use]
    pub fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::MacOs
        } else {
            Self::Linux
        }
    }
}

/// Where gh may be, in order: every `PATH` folder first, then the layout's usual install folders, the user's own
/// before the system's (as a login shell's `PATH` orders them).
#[must_use]
pub fn candidates_for(
    layout: InstallLayout,
    env: impl Fn(&str) -> Option<OsString>,
) -> Vec<PathBuf> {
    let name = if layout == InstallLayout::Windows {
        "gh.exe"
    } else {
        "gh"
    };
    let mut list: Vec<PathBuf> = env("PATH")
        .map(|p| std::env::split_paths(&p).map(|d| d.join(name)).collect())
        .unwrap_or_default();
    let home = |parts: &[&str]| {
        env(if layout == InstallLayout::Windows {
            "USERPROFILE"
        } else {
            "HOME"
        })
        .map(|h| parts.iter().fold(PathBuf::from(h), |p, part| p.join(part)))
    };
    match layout {
        InstallLayout::Windows => {
            for var in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
                if let Some(base) = env(var) {
                    list.push(PathBuf::from(&base).join("GitHub CLI").join(name));
                    list.push(
                        PathBuf::from(base)
                            .join("Programs")
                            .join("GitHub CLI")
                            .join(name),
                    );
                }
            }
            if let Some(base) = env("LOCALAPPDATA") {
                list.push(
                    PathBuf::from(base)
                        .join("Microsoft")
                        .join("WinGet")
                        .join("Links")
                        .join(name),
                );
            }
            if let Some(scoop) = env("SCOOP") {
                list.push(PathBuf::from(scoop).join("shims").join(name));
            }
            list.extend(home(&["scoop", "shims", name]));
        }
        InstallLayout::MacOs => {
            list.extend(home(&[".local", "bin", name]));
            for dir in ["/opt/homebrew/bin", "/usr/local/bin", "/opt/local/bin"] {
                list.push(PathBuf::from(dir).join(name));
            }
        }
        InstallLayout::Linux => {
            list.extend(home(&[".local", "bin", name]));
            list.extend(home(&[".linuxbrew", "bin", name]));
            for dir in [
                "/home/linuxbrew/.linuxbrew/bin",
                "/usr/local/bin",
                "/usr/bin",
                "/snap/bin",
            ] {
                list.push(PathBuf::from(dir).join(name));
            }
        }
    }
    list
}

/// What a finished gh call returned.
struct Output {
    ok: bool,
    stdout: Vec<u8>,
    stderr: String,
}

impl Gh {
    /// The program path (for the dialog: "using gh at …").
    #[must_use]
    pub fn program(&self) -> &Path {
        &self.program
    }

    fn run(
        &self,
        args: &[&str],
        stdin: Option<&[u8]>,
        timeout: Duration,
    ) -> Result<Output, GitError> {
        let mut command = vw_platform::process_command::hidden(&self.program);
        command.current_dir(&self.workdir).args(args);
        for (key, _) in std::env::vars_os() {
            if key
                .to_string_lossy()
                .to_ascii_uppercase()
                .starts_with("GIT_")
            {
                command.env_remove(key);
            }
        }
        command
            .env("GH_PROMPT_DISABLED", "1")
            .env("GH_NO_UPDATE_NOTIFIER", "1")
            .env("GH_SPINNER_DISABLED", "1")
            .env("GH_PAGER", "")
            .env("NO_COLOR", "1")
            .env("CLICOLOR", "0")
            .stdin(if stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .map_err(|e| GitError::Gh(format!("cannot start {}: {e}", self.program.display())))?;
        let writer = child
            .stdin
            .take()
            .zip(stdin.map(<[u8]>::to_vec))
            .map(|(mut pipe, bytes)| {
                std::thread::spawn(move || {
                    let _ = pipe.write_all(&bytes);
                })
            });
        let out = child.stdout.take().map(|p| reader(p, OUTPUT_CAP));
        let err = child.stderr.take().map(|p| reader(p, 16 << 10));
        let deadline = Instant::now() + timeout;
        let status = loop {
            if let Some(status) = child.try_wait().map_err(|e| GitError::Gh(e.to_string()))? {
                break status;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err(GitError::Gh(format!(
                    "{} took longer than {} s and was stopped",
                    args.first().copied().unwrap_or("gh"),
                    timeout.as_secs()
                )));
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        if let Some(w) = writer {
            let _ = w.join();
        }
        let stdout = out
            .map(|h| h.join().unwrap_or_default())
            .unwrap_or_default();
        let stderr = err
            .map(|h| h.join().unwrap_or_default())
            .unwrap_or_default();
        Ok(Output {
            ok: status.success(),
            stdout,
            stderr: String::from_utf8_lossy(&stderr).into_owned(),
        })
    }

    /// True when gh is signed in for `host` (`gh auth status --hostname`).
    ///
    /// # Errors
    /// gh cannot be started.
    pub fn signed_in(&self, host: &str) -> Result<bool, GitError> {
        Ok(self
            .run(&["auth", "status", "--hostname", host], None, READ_TIMEOUT)?
            .ok)
    }

    /// Creates the pull request and returns its address.
    ///
    /// # Errors
    /// `Rejected` with gh's words (a PR exists, no permission,...), gh failures.
    pub fn create(
        &self,
        forge: &Forge,
        head: &str,
        base: &str,
        title: &str,
        body: &str,
        draft: bool,
    ) -> Result<String, GitError> {
        if !names::valid_ref_name(head) || !names::valid_ref_name(base) {
            return Err(GitError::Blocked(
                "The branch name cannot be used with gh.".to_owned(),
            ));
        }
        // `--flag=value`: a value that starts with `-` (a title, a branch) is never read as an option.
        let repo = format!("--repo={}", forge.gh_repo());
        let head = format!("--head={head}");
        let base = format!("--base={base}");
        let title = format!("--title={title}");
        let mut args = vec!["pr", "create", &repo, &head, &base, &title, "--body-file=-"];
        if draft {
            args.push("--draft");
        }
        let out = self.run(&args, Some(body.as_bytes()), CREATE_TIMEOUT)?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        if !out.ok {
            return Err(GitError::Rejected(format!(
                "GitHub did not create the pull request: {}",
                clip(&out.stderr, 400)
            )));
        }
        // gh prints the new PR's address as the last line.
        stdout
            .lines()
            .rev()
            .map(str::trim)
            .find(|l| l.starts_with("https://"))
            .map(str::to_owned)
            .ok_or_else(|| GitError::Gh(format!("unexpected answer: {}", clip(&stdout, 200))))
    }

    /// The card for a head (`owner:branch` for a fork), or a pull request number (`None` when absent).
    ///
    /// # Errors
    /// gh failures other than "no pull requests found".
    pub fn view(&self, forge: &Forge, head: &str) -> Result<Option<PrCard>, GitError> {
        let valid = head.split_once(':').map_or_else(
            || names::valid_ref_name(head),
            |(owner, branch)| names::valid_ref_name(owner) && names::valid_ref_name(branch),
        );
        if !valid {
            return Err(GitError::Blocked(
                "The branch name cannot be used with gh.".to_owned(),
            ));
        }
        let repo = format!("--repo={}", forge.gh_repo());
        let fields = format!("--json={VIEW_FIELDS}");
        let out = self.run(
            &["pr", "view", &repo, &fields, "--", head],
            None,
            READ_TIMEOUT,
        )?;
        if !out.ok {
            let lower = out.stderr.to_ascii_lowercase();
            if lower.contains("no pull requests found") || lower.contains("could not resolve") {
                return Ok(None);
            }
            return Err(GitError::Gh(clip(&out.stderr, 300)));
        }
        let value: Value = serde_json::from_slice(&out.stdout)
            .map_err(|e| GitError::Gh(format!("unreadable answer: {e}")))?;
        Ok(pr::from_gh(&value))
    }

    /// Every pull request (any state, newest first, at most 30) whose head is `head`.
    ///
    /// # Errors
    /// gh failures, or a branch name gh cannot be given.
    pub fn list(&self, forge: &Forge, head: &str) -> Result<Vec<PrCard>, GitError> {
        if !names::valid_ref_name(head) {
            return Err(GitError::Blocked(
                "The branch name cannot be used with gh.".to_owned(),
            ));
        }
        let repo = format!("--repo={}", forge.gh_repo());
        let fields = format!("--json={VIEW_FIELDS}");
        let head = format!("--head={head}");
        let out = self.run(
            &[
                "pr",
                "list",
                &repo,
                &head,
                "--state=all",
                "--limit=30",
                &fields,
            ],
            None,
            READ_TIMEOUT,
        )?;
        if !out.ok {
            return Err(GitError::Gh(clip(&out.stderr, 300)));
        }
        let value: Value = serde_json::from_slice(&out.stdout)
            .map_err(|e| GitError::Gh(format!("unreadable answer: {e}")))?;
        Ok(value
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(pr::from_gh)
            .collect())
    }

    /// The names of pull request `number`'s required checks (`gh pr checks --required`). gh ends with a non-zero
    /// status while checks fail or run, so the answer is read whatever the status; "no required checks" is an
    /// empty list.
    ///
    /// # Errors
    /// gh failures without a readable answer.
    pub fn required_checks(&self, forge: &Forge, number: u64) -> Result<Vec<String>, GitError> {
        let repo = format!("--repo={}", forge.gh_repo());
        let number = number.to_string();
        let out = self.run(
            &[
                "pr",
                "checks",
                &repo,
                "--required",
                "--json=name",
                "--",
                &number,
            ],
            None,
            READ_TIMEOUT,
        )?;
        if let Ok(value) = serde_json::from_slice::<Value>(&out.stdout) {
            return Ok(value
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|c| c.get("name").and_then(Value::as_str).map(str::to_owned))
                .collect());
        }
        if out
            .stderr
            .to_ascii_lowercase()
            .contains("no required checks")
        {
            return Ok(Vec::new());
        }
        Err(GitError::Gh(clip(&out.stderr, 300)))
    }

    /// The raw `gh run view --job <job> --log-failed` output of one GitHub Actions job; the
    /// caller keeps its tail (`checklog::tail_of_gh_log`).
    ///
    /// # Errors
    /// gh failures (not signed in, no such job, a log GitHub no longer keeps).
    pub fn failed_log(&self, forge: &Forge, job: u64) -> Result<String, GitError> {
        let repo = format!("--repo={}", forge.gh_repo());
        let job = format!("--job={job}");
        let out = self.run(
            &["run", "view", &repo, &job, "--log-failed"],
            None,
            READ_TIMEOUT,
        )?;
        if !out.ok {
            return Err(GitError::Gh(clip(&out.stderr, 300)));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// The review comments of pull request `number` (top-level and on lines), read from GitHub's REST API through
    /// `gh api` so bot accounts can be told apart (`user.type`), and why the line comments could
    /// not be read when that part failed (the message then says so instead of leaving them out silently).
    ///
    /// # Errors
    /// gh failures reading the reviews or the conversation comments.
    pub fn comments(
        &self,
        forge: &Forge,
        number: u64,
    ) -> Result<(Vec<ReviewComment>, Option<String>), GitError> {
        let host_flag = format!("--hostname={}", forge.host);
        let api = |endpoint: String| -> Result<Output, GitError> {
            let mut args = vec!["api"];
            if forge.host != "github.com" {
                args.push(host_flag.as_str());
            }
            args.extend(["--", endpoint.as_str()]);
            self.run(&args, None, READ_TIMEOUT)
        };
        let json = |out: Output| -> Result<Value, GitError> {
            if !out.ok {
                return Err(GitError::Gh(clip(&out.stderr, 300)));
            }
            serde_json::from_slice(&out.stdout)
                .map_err(|e| GitError::Gh(format!("unreadable answer: {e}")))
        };
        let base = format!("repos/{}", forge.path);
        let reviews = json(api(format!("{base}/pulls/{number}/reviews?per_page=100"))?)?;
        let talk = json(api(format!(
            "{base}/issues/{number}/comments?per_page=100"
        ))?)?;
        let top = serde_json::json!({"reviews": reviews, "comments": talk});
        let (lines, unread) =
            match api(format!("{base}/pulls/{number}/comments?per_page=100")).and_then(json) {
                Ok(lines) => (Some(lines), None),
                Err(e) => (None, Some(format!("gh could not read them: {e}"))),
            };
        Ok((pr::comments_from(Some(&top), lines.as_ref()), unread))
    }
}

#[cfg(test)]
#[path = "gh_tests.rs"]
mod tests;

fn reader(mut pipe: impl Read + Send + 'static, cap: usize) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut kept = Vec::new();
        let mut buf = vec![0_u8; 32 * 1024];
        loop {
            match pipe.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let room = cap.saturating_sub(kept.len());
                    kept.extend_from_slice(&buf[..n.min(room)]);
                }
            }
        }
        kept
    })
}
