//! Synthetic worktree routes in the scripted fake server.
//! - `session/start` with `agentConfig.worktree = {kind: "create", branch, name}` makes the worktree right
//!   after the answer (Vibe's deferred task), at `$VIBE_HOME/worktrees/<repo>-fake/<name>` with plain git
//!   (`worktree add` of the existing branch, else `-b` from `HEAD`), holds it for the session, moves the
//!   session's `cwd` there and publishes `session/updated` with `/cwd` and `/worktree` (Vibe's
//!   `_announce_worktree`) plus a completed `worktree` effect entry.
//! - `workspace/git/worktrees/remove`: `kept_unmanaged` for a folder it did not make, `not_found` for a gone
//!   one; `inspect: true` probes (`kept_in_use` when another session holds it, `kept_dirty` with Vibe's
//!   reasons, else the would-be `removed`); `force: true` removes without a copy (deleting the branch when
//!   `deleteBranch` says so, default: when it created the branch). Every call is appended to
//!   `$VIBE_HOME/worktree-remove.log` (one JSON per line) for route assertions.
//! - A session's hold goes when its channel closes or it is deleted; `workspace/git/worktrees/list` lists
//!   what it made.
//! - `workspace/git/checkouts` (the fixture): one checkout per `repoLocalPaths` entry, shaped like Vibe's
//!   `WorkspaceGitCheckout` (`protocol.py:1753`), with the repository root, a worktree it made in that
//!   repository (if any), the current branch and a remote address carrying a user name and secret (so a test
//!   can prove none of it reaches a client that must not see it).
//!
//! Test code: plain git, no hardening (the repositories are the tests' own).

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use serde_json::{Value, json};

use crate::script::Script;

/// Methods this module answers.
pub const HANDLES: &[&str] = &[
    "workspace/git/worktrees/remove",
    "workspace/git/worktrees/list",
    "workspace/git/checkouts",
];

/// One worktree the fake made (Vibe's claim record).
#[derive(Clone, Debug)]
pub struct Claim {
    pub repo_root: PathBuf,
    pub branch: String,
    pub branch_created: bool,
    pub base_commit: String,
    pub holders: Vec<String>,
}

/// The claims, keyed by the worktree root.
pub type Claims = BTreeMap<PathBuf, Claim>;

pub(crate) fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

fn canonical(p: &str) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| PathBuf::from(p))
}

fn log_remove(params: &Value) {
    let Some(home) = std::env::var_os("VIBE_HOME") else {
        return;
    };
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(Path::new(&home).join("worktree-remove.log"))
    {
        let _ = writeln!(f, "{params}");
    }
}

impl Script {
    /// After a `session/start` answer: raise the requested worktree (a short delay, like Vibe's task).
    pub fn raise_worktree(self: &Arc<Self>, result: &Value, params: &Value) {
        let Some(wt) = params
            .pointer("/agentConfig/worktree")
            .filter(|w| w["kind"] == "create")
        else {
            return;
        };
        let (Some(sid), Some(cwd)) = (
            result.pointer("/state/session/id").and_then(Value::as_str),
            params.pointer("/agentConfig/cwd").and_then(Value::as_str),
        ) else {
            return;
        };
        let (sid, cwd) = (sid.to_owned(), cwd.to_owned());
        self.world.lock().unwrap().raising.insert(sid.clone());
        let branch = wt["branch"].as_str().unwrap_or_default().to_owned();
        let name = wt["name"].as_str().unwrap_or_default().to_owned();
        let this = self.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(50));
            this.make_worktree(&sid, Path::new(&cwd), &branch, &name);
            this.world.lock().unwrap().raising.remove(&sid);
        });
    }

    /// Waits (at most 10 s) while `sid`'s worktree is being made.
    pub fn wait_for_worktree(&self, sid: &str) {
        for _ in 0..500 {
            if !self.world.lock().unwrap().raising.contains(sid) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    fn make_worktree(&self, sid: &str, cwd: &Path, branch: &str, name: &str) {
        let Some(root) = git(cwd, &["rev-parse", "--show-toplevel"]) else {
            return;
        };
        let root = PathBuf::from(root);
        let home = std::env::var_os("VIBE_HOME").map_or_else(std::env::temp_dir, PathBuf::from);
        let repo_name = root
            .file_name()
            .map_or("repo".into(), |n| n.to_string_lossy().into_owned());
        let target = home
            .join("worktrees")
            .join(format!("{repo_name}-fake"))
            .join(name);
        let _ = std::fs::create_dir_all(target.parent().unwrap_or(&home));
        let exists = git(
            &root,
            &[
                "show-ref",
                "--verify",
                "-q",
                &format!("refs/heads/{branch}"),
            ],
        )
        .is_some();
        let t = target.to_string_lossy().into_owned();
        let added = if exists {
            git(&root, &["worktree", "add", "-q", &t, branch])
        } else {
            git(&root, &["worktree", "add", "-q", "-b", branch, &t])
        };
        if added.is_none() {
            return;
        }
        let target = canonical(&t);
        let base = git(&target, &["rev-parse", "HEAD"]).unwrap_or_default();
        self.world.lock().unwrap().claims.insert(
            target.clone(),
            Claim {
                repo_root: root,
                branch: branch.to_owned(),
                branch_created: !exists,
                base_commit: base,
                holders: vec![sid.to_owned()],
            },
        );
        let path = target.to_string_lossy().into_owned();
        let worktree = json!({"name": name, "branch": branch, "path": path, "created": true});
        let (p, w) = (path.clone(), worktree.clone());
        self.emit(
            sid,
            "session/updated",
            json!({"patch": [{"op": "replace", "path": "/cwd", "value": path},
                             {"op": "replace", "path": "/worktree", "value": worktree}]}),
            move |s| {
                s["session"]["cwd"] = json!(p);
                s["session"]["worktree"] = w;
            },
        );
        let id = self.next("worktree");
        self.add_entry(
            sid,
            json!({"id": id, "type": "effect", "sessionId": sid, "turnId": null, "generationStatus": "completed",
                "createdAt": 1, "updatedAt": 1, "title": "worktree",
                "detail": {"kind": "worktree", "toolName": "worktree", "input": {"name": name, "branch": branch, "path": target},
                    "display": {"verb": "Creating", "message": name, "settledVerb": "Created",
                        "settledMessage": format!("{name} on {branch}")}},
                "state": {"status": "completed", "display": {"success": true, "verb": "Created",
                    "message": format!("{name} on {branch}")}}}),
        );
    }

    /// Drops `sid`'s holds (its channel closed, or it was deleted).
    pub fn release_holds(&self, sid: &str) {
        for claim in self.world.lock().unwrap().claims.values_mut() {
            claim.holders.retain(|h| h != sid);
        }
    }

    /// `workspace/git/checkouts`: see the module docs.
    fn checkouts(&self, params: &Value) -> Value {
        let claims = self.world.lock().unwrap().claims.clone();
        let items: Vec<Value> = params["repoLocalPaths"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .filter_map(Value::as_str)
            .map(|p| {
                let root = canonical(p);
                let worktree = claims
                    .iter()
                    .find(|(_, c)| canonical(&c.repo_root.display().to_string()) == root)
                    .map(|(path, _)| path.clone());
                json!({"repoLocalPath": p, "ok": true, "isPrimary": true,
                    "repoUrl": "https://user:secret@example.invalid/repo.git",
                    "root": root, "worktree": worktree,
                    "branch": git(&root, &["rev-parse", "--abbrev-ref", "HEAD"]),
                    "baseBranch": "main", "message": null})
            })
            .collect();
        json!({"checkouts": items})
    }

    /// `workspace/git/worktrees/remove`, `.../list` and `workspace/git/checkouts`.
    pub fn worktrees_call(&self, conn: u64, method: &str, params: &Value) -> Value {
        if method == "workspace/git/checkouts" {
            return self.checkouts(params);
        }
        if method == "workspace/git/worktrees/list" {
            let claims = self.world.lock().unwrap().claims.clone();
            let items: Vec<Value> = claims
                .iter()
                .map(|(p, c)| {
                    json!({"name": p.file_name().map(|n| n.to_string_lossy().into_owned()),
                    "branch": c.branch, "cwd": p, "root": p, "repoRoot": c.repo_root})
                })
                .collect();
            return json!({"worktrees": items});
        }
        log_remove(params);
        let path = canonical(params["cwd"].as_str().unwrap_or_default());
        let own = self.world.lock().unwrap().attached.get(&conn).cloned();
        let Some(claim) = self.world.lock().unwrap().claims.get(&path).cloned() else {
            return json!({"outcome": "kept_unmanaged"});
        };
        if !path.is_dir() {
            self.world.lock().unwrap().claims.remove(&path);
            return json!({"outcome": "not_found"});
        }
        let others: Vec<&String> = claim
            .holders
            .iter()
            .filter(|h| Some(*h) != own.as_ref())
            .collect();
        let holders = others.len();
        let flag = |k: &str| params.get(k).and_then(Value::as_bool);
        let inspect = flag("inspect") == Some(true);
        let force = flag("force") == Some(true);
        if holders > 0 {
            return json!({"outcome": "kept_in_use", "branch": claim.branch, "holders": holders,
                "branchCreated": claim.branch_created});
        }
        let status =
            git(&path, &["status", "--porcelain", "--untracked-files=all"]).unwrap_or_default();
        let ahead: u64 = git(
            &path,
            &[
                "rev-list",
                "--count",
                &format!("{}..HEAD", claim.base_commit),
            ],
        )
        .and_then(|n| n.parse().ok())
        .unwrap_or(0);
        let mut reasons = Vec::new();
        if status.lines().any(|l| !l.starts_with("??")) {
            reasons.push("uncommitted changes".to_owned());
        }
        if status.lines().any(|l| l.starts_with("??")) {
            reasons.push("untracked files".to_owned());
        }
        if ahead > 0 {
            reasons.push(format!("{ahead} commit(s) added during this session"));
        }
        if inspect {
            let outcome = if reasons.is_empty() {
                "removed"
            } else {
                "kept_dirty"
            };
            return json!({"outcome": outcome, "root": path, "branch": claim.branch, "reasons": reasons,
                "branchCreated": claim.branch_created, "holders": holders});
        }
        if !force {
            #[cfg(feature = "fixture-extensions")]
            return self.private_default_worktree_remove(&path, &claim);
            #[cfg(not(feature = "fixture-extensions"))]
            return json!({"outcome": "kept_error", "reasons": ["explicit removal required"]});
        }
        let delete_branch = flag("deleteBranch").unwrap_or(claim.branch_created);
        let p = path.to_string_lossy().into_owned();
        if git(&claim.repo_root, &["worktree", "remove", "--force", &p]).is_none() {
            return json!({"outcome": "kept_error", "reasons": ["git worktree remove failed"]});
        }
        let deleted =
            delete_branch && git(&claim.repo_root, &["branch", "-D", &claim.branch]).is_some();
        self.world.lock().unwrap().claims.remove(&path);
        json!({"outcome": "removed", "root": path, "branch": claim.branch, "branchDeleted": deleted,
            "reasons": [], "branchCreated": claim.branch_created, "holders": 0})
    }
}
