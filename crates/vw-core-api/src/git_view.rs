//! The `git` command's requests and results: what the branch bar, the Commit / Push / PR dialogs, the
//! PR card and "Bring into main" send and get back, from the window and the Debug API alike. Logic lives in
//! `git_cmd.rs` (status, commit, settings) and `git_publish.rs` (push, PR, card, bring in); the git and forge
//! side in the vw-git crate.
//!
//! Depends on: vw-git (views), serde. Called by: `command` (the `git` variant), `git_cmd`, `git_publish`, the
//! UI through the generated TypeScript.
//! Invariants: wire form `{"op": "<camelCase>", ...camelCase fields}`, unknown fields refused; no view carries a
//! credential (remote addresses are shown without user info).

use serde::{Deserialize, Serialize};
use vw_git::commit::CommitFile;
use vw_git::forge::Forge;
use vw_git::hooks::HookInfo;
use vw_git::merge::BringPlan;
use vw_git::pr::PrCard;
use vw_git::push::PushPlan;
use vw_git::state::{BranchState, RemoteInfo};

/// One `git` request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "op",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum GitRequest {
    /// The branch bar: branch, base, counts, the cached PR card. Reads only, no network.
    Status {
        /// Session id.
        session_id: String,
        #[serde(default)]
        #[cfg_attr(feature = "ts", ts(optional))]
        /// Repository root relative to the session folder (default: session repository).
        repo: Option<String>,
    },
    /// What a commit would contain now (the dialog).
    PrepareCommit {
        /// Session id.
        session_id: String,
        #[serde(default)]
        #[cfg_attr(feature = "ts", ts(optional))]
        /// Repository root relative to the session folder (default: session repository).
        repo: Option<String>,
    },
    /// A commit message draft for `paths` (Mistral with the user's key, else the session title).
    DraftMessage {
        /// Session id.
        session_id: String,
        #[serde(default)]
        #[cfg_attr(feature = "ts", ts(optional))]
        /// Repository root relative to the session folder (default: session repository).
        repo: Option<String>,
        /// The files that will be committed.
        paths: Vec<String>,
    },
    /// Commits `paths` with `message` on `branch`, if `HEAD` is still `expectedHead`.
    Commit {
        /// Session id.
        session_id: String,
        #[serde(default)]
        #[cfg_attr(feature = "ts", ts(optional))]
        /// Repository root relative to the session folder (default: session repository).
        repo: Option<String>,
        /// Shown paths from `prepareCommit`.
        paths: Vec<String>,
        /// The message as the user left it.
        message: String,
        /// The branch the dialog showed.
        branch: String,
        /// `HEAD` when the dialog was built (`null` = no commit yet).
        expected_head: Option<String>,
        /// Run the repository's commit hooks (the dialog's switch).
        run_hooks: bool,
    },
    /// What a push would do (remote, address, branch, commits).
    PushPlan {
        /// Session id.
        session_id: String,
        #[serde(default)]
        #[cfg_attr(feature = "ts", ts(optional))]
        /// Repository root relative to the session folder (default: session repository).
        repo: Option<String>,
        /// The remote (default: the upstream's, `origin`, or the only one).
        #[serde(default)]
        #[cfg_attr(feature = "ts", ts(optional))]
        remote: Option<String>,
    },
    /// Pushes after the native confirmation (the window; the Debug API acts as the user). `commit` is the
    /// commit the plan showed.
    Push {
        /// Session id.
        session_id: String,
        #[serde(default)]
        #[cfg_attr(feature = "ts", ts(optional))]
        /// Repository root relative to the session folder (default: session repository).
        repo: Option<String>,
        /// The remote.
        remote: String,
        /// The commit the user saw.
        commit: String,
    },
    /// What creating a pull request would do: forge, gh or the browser, base, title, body, push first.
    PrPlan {
        /// Session id.
        session_id: String,
        #[serde(default)]
        #[cfg_attr(feature = "ts", ts(optional))]
        /// Repository root relative to the session folder (default: session repository).
        repo: Option<String>,
        /// The remote.
        #[serde(default)]
        #[cfg_attr(feature = "ts", ts(optional))]
        remote: Option<String>,
    },
    /// Pushes when needed and creates the pull request (gh) or returns the forge's page (browser), after one
    /// native confirmation when anything leaves the machine.
    CreatePr {
        /// Session id.
        session_id: String,
        #[serde(default)]
        #[cfg_attr(feature = "ts", ts(optional))]
        /// Repository root relative to the session folder (default: session repository).
        repo: Option<String>,
        /// The remote.
        remote: String,
        /// The base branch on the forge.
        base: String,
        /// The title.
        title: String,
        /// The body.
        body: String,
        /// A draft pull request (gh only; on GitHub's page the user picks it).
        draft: bool,
        /// The commit the user saw.
        commit: String,
    },
    /// The PR card. `refresh` asks the forge now (subject to the floor between asks), else the cached card.
    PrStatus {
        /// Session id.
        session_id: String,
        #[serde(default)]
        #[cfg_attr(feature = "ts", ts(optional))]
        /// Repository root relative to the session folder (default: session repository).
        repo: Option<String>,
        /// Ask the forge now.
        refresh: bool,
    },
    /// A prepared composer message (never sent by the core).
    PrepareMessage {
        /// Session id.
        session_id: String,
        #[serde(default)]
        #[cfg_attr(feature = "ts", ts(optional))]
        /// Repository root relative to the session folder (default: session repository).
        repo: Option<String>,
        /// Which one.
        what: PreparedKind,
    },
    /// The dry run of "Bring into <base>" for a session copy.
    BringPlan {
        /// Session id.
        session_id: String,
        #[serde(default)]
        #[cfg_attr(feature = "ts", ts(optional))]
        /// Repository root relative to the session folder (default: session repository).
        repo: Option<String>,
    },
    /// Brings the copy's branch into its base after the native confirmation, if both are where the plan saw
    /// them.
    BringIn {
        /// Session id.
        session_id: String,
        #[serde(default)]
        #[cfg_attr(feature = "ts", ts(optional))]
        /// Repository root relative to the session folder (default: session repository).
        repo: Option<String>,
        /// The base's commit the plan showed.
        base_commit: String,
        /// The branch's commit the plan showed.
        branch_commit: String,
    },
    /// The per-project settings of the session's repository.
    Settings {
        /// Session id.
        session_id: String,
        #[serde(default)]
        #[cfg_attr(feature = "ts", ts(optional))]
        /// Repository root relative to the session folder (default: session repository).
        repo: Option<String>,
    },
    /// Changes the per-project settings (only the fields given).
    SetSettings {
        /// Session id.
        session_id: String,
        #[serde(default)]
        #[cfg_attr(feature = "ts", ts(optional))]
        /// Repository root relative to the session folder (default: session repository).
        repo: Option<String>,
        /// The commit message instruction.
        #[serde(default)]
        #[cfg_attr(feature = "ts", ts(optional))]
        instruction: Option<String>,
        /// Add Vibe's credit line to commit messages.
        #[serde(default)]
        #[cfg_attr(feature = "ts", ts(optional))]
        credit: Option<bool>,
        /// Run the repository's commit hooks on Commit.
        #[serde(default)]
        #[cfg_attr(feature = "ts", ts(optional))]
        run_hooks: Option<bool>,
    },
}

/// A prepared composer message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum PreparedKind {
    /// "Send failing checks to Vibe".
    Checks,
    /// "Send review comments to Vibe".
    Review,
    /// "Ask Vibe to update the copy from <base>".
    UpdateFromBase,
}

/// Where the session works.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum GitLocation {
    /// Its own copy (a worktree Vibe Work made).
    Copy,
    /// The folder itself.
    Folder,
}

/// The branch bar.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct GitStatusView {
    /// Session id.
    pub session_id: String,
    /// Repositories discovered from this session's recorded file changes.
    pub repos: Vec<GitRepoView>,
    /// Selected repository root, relative to the session repository.
    pub repo: Option<String>,
    /// The folder is in a git repository and the bar can show.
    pub available: bool,
    /// Why not (plain words).
    pub reason: Option<String>,
    /// Copy or folder.
    pub location: Option<GitLocation>,
    /// Branch, remotes, upstream, base.
    pub state: Option<BranchState>,
    /// Files a commit would contain (`null` when they could not be listed: `blocker` says why).
    pub uncommitted: Option<u32>,
    /// Commits a default push would send (including a branch without an upstream), when known.
    pub to_push: Option<u32>,
    /// Why commits are not possible here (a repository setting, an operation in progress, detached).
    pub blocker: Option<String>,
    /// The agent's turn is running (Commit waits for it).
    pub running: bool,
    /// The last known PR card.
    pub pr: Option<PrCard>,
}

/// One repository available to the branch bar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct GitRepoView {
    /// Display name of the repository folder.
    pub name: String,
    /// Root relative to the session folder (`.` for its repository).
    pub root: String,
    /// Current branch, when attached.
    pub branch: Option<String>,
}

/// The Commit dialog.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct CommitPlanView {
    /// Session id.
    pub session_id: String,
    /// Copy or folder.
    pub location: GitLocation,
    /// The branch that gets the commit.
    pub branch: String,
    /// `HEAD` now (`null` = no commit yet).
    pub head: Option<String>,
    /// Every file a commit of everything would contain.
    pub files: Vec<CommitFile>,
    /// The author git will write, when known.
    pub author: Option<String>,
    /// Why git has no author yet (how to set it).
    pub author_problem: Option<String>,
    /// The commit hooks that will run when the switch is on.
    pub hooks: HookInfo,
    /// The project's switch.
    pub run_hooks: bool,
    /// The project's credit-line switch.
    pub credit: bool,
    /// The project's instruction for drafted messages (empty: none).
    pub instruction: String,
    /// A note for the user (a "this folder" session commits the whole folder).
    pub note: Option<String>,
    /// Why Commit is not possible now.
    pub blocker: Option<String>,
}

/// Where a commit message came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum DraftSource {
    /// Written by Mistral.
    Model,
    /// Built from the session title.
    Fallback,
}

/// The Push dialog.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct PushPlanView {
    /// What will be pushed.
    pub plan: PushPlan,
    /// The sentence the confirmation shows.
    pub sentence: String,
    /// The remotes to choose from.
    pub remotes: Vec<RemoteInfo>,
}

/// How a pull request is made.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum PrVia {
    /// GitHub's CLI, signed in.
    Gh,
    /// The forge's page in the system browser.
    Browser,
    /// No supported forge page: Create PR is blocked.
    None,
}

/// The PR dialog.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct PrPlanView {
    /// The remote.
    pub remote: String,
    /// Its forge, when known.
    pub forge: Option<Forge>,
    /// gh, the browser, or neither.
    pub via: PrVia,
    /// Why gh is not used (not installed, not signed in), for the dialog.
    pub gh_note: Option<String>,
    /// The base branch on the forge.
    pub base: String,
    /// The head branch.
    pub head: String,
    /// The drafted title.
    pub title: String,
    /// The drafted body (the template filled in when the repository has one).
    pub body: String,
    /// The template's path, when one was used.
    pub template: Option<String>,
    /// The push that has to happen first, if any.
    pub push: Option<PushPlan>,
    /// A pull request that already exists for this branch.
    pub existing: Option<PrCard>,
    /// The sentence the confirmation shows.
    pub sentence: String,
}

/// The result of `createPr`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct PrCreatedView {
    /// How it was made.
    pub via: PrVia,
    /// The pull request (gh) or the page to open (browser; `null` for `none`).
    pub url: Option<String>,
    /// What happened, in words.
    pub message: String,
}

/// Where the PR card's data came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum PrSource {
    /// gh.
    Gh,
    /// GitHub's public REST API (public repositories, 60 requests an hour).
    PublicApi,
    /// Nothing could be read (the card offers the forge's page).
    None,
}

/// The PR card.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct PrStatusView {
    /// The pull request, when one is known.
    pub card: Option<PrCard>,
    /// Where it came from.
    pub source: PrSource,
    /// When the forge was last asked (ms since the epoch; 0 = never).
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub fetched_at_ms: u64,
    /// Rate limit, private repository, no forge (plain words).
    pub note: Option<String>,
    /// The forge page for this branch's pull requests.
    pub open_url: Option<String>,
}

/// The "Bring into <base>" dialog.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct BringPlanView {
    /// The dry run.
    pub plan: BringPlan,
    /// The sentence the confirmation shows.
    pub sentence: String,
}

/// Per-project settings (by repository).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct GitSettings {
    /// The instruction the commit message model follows.
    pub instruction: String,
    /// Add "Co-Authored-By: Mistral Vibe <vibe@mistral.ai>" to commit messages.
    pub credit: bool,
    /// Run the repository's commit hooks on Commit (default on).
    pub run_hooks: bool,
}

impl Default for GitSettings {
    fn default() -> Self {
        Self {
            instruction: String::new(),
            credit: false,
            run_hooks: true,
        }
    }
}

/// One `git` answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum GitResult {
    /// `status`.
    Status {
        /// The bar.
        status: GitStatusView,
    },
    /// `prepareCommit`.
    CommitPlan {
        /// The dialog.
        plan: CommitPlanView,
    },
    /// `draftMessage`.
    Draft {
        /// The message.
        message: String,
        /// Where it came from.
        source: DraftSource,
        /// Why the model was not used, when it was not.
        note: Option<String>,
    },
    /// `commit`.
    Committed {
        /// The new commit.
        commit: String,
        /// What git and the hooks printed.
        output: String,
        /// Files that left the review because they were committed as shown.
        kept: u32,
    },
    /// `pushPlan`.
    PushPlan {
        /// The dialog.
        plan: PushPlanView,
    },
    /// `push`.
    Pushed {
        /// What happened.
        message: String,
    },
    /// `prPlan`.
    PrPlan {
        /// The dialog.
        plan: PrPlanView,
    },
    /// `createPr`.
    PrCreated {
        /// What happened.
        created: PrCreatedView,
    },
    /// `prStatus`.
    PrStatus {
        /// The card.
        status: PrStatusView,
    },
    /// `prepareMessage`.
    Prepared {
        /// The composer text.
        text: String,
    },
    /// `bringPlan`.
    BringPlan {
        /// The dialog.
        plan: BringPlanView,
    },
    /// `bringIn`.
    Brought {
        /// The base's new commit.
        commit: String,
    },
    /// `settings` and `setSettings`.
    Settings {
        /// The settings.
        settings: GitSettings,
    },
}
