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

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::error::{GitError, clip};
use crate::forge::Forge;
use crate::names;
use crate::pr::{self, PrCard, ReviewComment};

/// The JSON fields the card reads.
pub const VIEW_FIELDS: &str = "number,url,title,state,isDraft,reviewDecision,mergeable,statusCheckRollup,headRefName,baseRefName";

const OUTPUT_CAP: usize = 8 << 20;
const READ_TIMEOUT: Duration = Duration::from_secs(45);
const CREATE_TIMEOUT: Duration = Duration::from_secs(120);

/// Where gh is and the neutral folder its calls run in.
#[derive(Debug, Clone)]
pub struct Gh {
    program: PathBuf,
    workdir: PathBuf,
}

/// The installed gh: `explicit` when given (tests), else the first `gh` on `PATH` or in the usual install
/// folders (a desktop app on macOS starts with a short `PATH`). `workdir` must be a private, existing
/// folder of ours.
#[must_use]
pub fn find(explicit: Option<&Path>, workdir: &Path) -> Option<Gh> {
    let program = match explicit {
        Some(p) => p.is_file().then(|| p.to_path_buf())?,
        None => candidates().into_iter().find(|p| p.is_file())?,
    };
    Some(Gh {
        program,
        workdir: workdir.to_path_buf(),
    })
}

fn candidates() -> Vec<PathBuf> {
    let name = if cfg!(windows) { "gh.exe" } else { "gh" };
    let mut list: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).map(|d| d.join(name)).collect())
        .unwrap_or_default();
    if cfg!(windows) {
        for var in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
            if let Some(base) = std::env::var_os(var) {
                list.push(PathBuf::from(&base).join("GitHub CLI").join(name));
                list.push(
                    PathBuf::from(base)
                        .join("Programs")
                        .join("GitHub CLI")
                        .join(name),
                );
            }
        }
    } else {
        for dir in [
            "/opt/homebrew/bin",
            "/usr/local/bin",
            "/usr/bin",
            "/snap/bin",
        ] {
            list.push(PathBuf::from(dir).join(name));
        }
        if let Some(home) = std::env::var_os("HOME") {
            list.push(PathBuf::from(home).join(".local").join("bin").join(name));
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
        let mut command = Command::new(&self.program);
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
        no_window(&mut command);
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

    /// The card of the newest pull request whose head is `head` (`None` when there is none).
    ///
    /// # Errors
    /// gh failures other than "no pull requests found".
    pub fn view(&self, forge: &Forge, head: &str) -> Result<Option<PrCard>, GitError> {
        if !names::valid_ref_name(head) {
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

#[cfg(windows)]
fn no_window(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    /// `CREATE_NO_WINDOW`: no console window flashes for each gh call.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn no_window(_command: &mut Command) {}
