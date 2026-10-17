//! The connector rule treats a write to a code forge like a git push: a Mistral connector tool that WRITES to
//! a code forge (GitHub, GitLab, Bitbucket, Gitea/Forgejo, Azure
//! DevOps, ...) is the user's button in Vibe Work, in every access mode, like `git push`. Read-only connector tools
//! stay available; connectors that are not forges (Slack, Google Drive, web search, ...) are not touched.
//!
//! Enforcement points:
//! - **The model proxy** (`vw-keyproxy`, the enforcement point): every connector call goes to
//!   `<api base>/v1/connectors-gateway/<connector id>/mcp` with the proxy token, so it passes our loopback proxy,
//!   which alone holds the key. A `tools/call` this module refuses is answered there, never forwarded, with
//!   [`CONNECTOR_REASON`] as the tool's text, in an ordinary result so the model reads the reason.
//! - **The `pre_tool` hook** (`hook.rs`, fallback): when Vibe's provider is not our proxy, the hook denies the call.
//!
//! The rule, on the connector's alias (Vibe's normalized connector name, `github_app`) and the remote tool's name:
//! - [`is_forge`]: the alias names a forge (a [`FORGE_NAMES`] entry as whole words of its [`normalize`]d form);
//! - [`is_read_only`]: the tool's verb is a read ([`READ_VERBS`] first, `get_commit`; or [`READ_SUFFIXES`] last,
//!   `pull_request_read`) and no word is a write word ([`WRITE_WORDS`]). Everything else on a forge is refused, so an
//!   unknown verb fails closed (`run_secret_scanning`, `request_copilot_review`, a future `transfer_x`).
//!
//! Tool name forms: unified `connector_<alias>.<tool>` (what the hook payload names, checked live), legacy
//! `connector_<alias>_<tool>` (the legacy harness and Vibe's `enabled_tools` names; the alias/tool split is then
//! ambiguous: any forge word makes it a forge call, and it reads when a split after that word reads).
//!
//! A guard against mistakes, not a sandbox: an agent that rewrites Vibe's connector cache or crafts its own
//! gateway requests with the proxy token is out of scope: the agent runs with the user's permissions.
//!
//! Depends on: nothing. Called by: `hook`, vw-keyproxy (`connectors.rs`), tests.
//! Invariant: pure functions over names; no I/O.

/// The refusal text the agent reads: the outcome first, then what to do (same voice as `hook::PUSH_REASON`).
pub const CONNECTOR_REASON: &str = "Not run: changing a repository through a connector (pushing files, creating or \
                                    merging pull requests, branches, issues, comments or repositories) is the \
                                    user's step in Vibe Work, like git push. Reading through the connector still \
                                    works. Commit your work and name its folder for the user: select that folder in \
                                    the Changes tab, then press Push or Create PR.";

/// Names that make a connector a code forge, in normalized form ([`normalize`]: lower case, words joined by one
/// `_`). Matched as whole words anywhere in the normalized name, multi-word entries included (`azure_devops`).
pub const FORGE_NAMES: &[&str] = &[
    "github",
    "gitlab",
    "bitbucket",
    "gitea",
    "forgejo",
    "codeberg",
    "gogs",
    "sourcehut",
    "srht",
    "gerrit",
    "azure_devops",
    "azure_dev_ops",
    "azuredevops",
    "azure_repos",
    "bit_bucket",
    "code_berg",
    "forge_jo",
    "source_hut",
    "ado",
    "git",
];

/// First words that make a tool a read (`get_commit`, `list_branches`, `search_code`).
pub const READ_VERBS: &[&str] = &[
    "get", "list", "search", "read", "fetch", "find", "view", "show", "describe", "query",
    "download", "compare", "lookup", "browse", "inspect", "diff", "count",
];

/// Last words that make a tool a read when the verb comes last (`pull_request_read`, `issue_read`).
pub const READ_SUFFIXES: &[&str] = &["read", "get", "list", "search", "view"];

/// Words that refuse a tool wherever they stand, also next to a read verb (`get_or_create_branch`). Nouns that are
/// also verbs (`commit`, `release`, `tag`, `run`, `label`) are not here, so `get_commit` stays a read; a tool whose
/// FIRST word is one of them is not a read anyway (no read verb) and is refused.
pub const WRITE_WORDS: &[&str] = &[
    "create", "update", "delete", "remove", "push", "write", "upload", "edit", "add", "merge",
    "set", "put", "post", "patch", "fork", "close", "reopen", "approve", "dismiss", "submit",
    "transfer", "archive", "rename", "move", "revert", "rebase", "squash", "publish", "dispatch",
    "trigger", "execute", "invite", "assign", "unassign", "lock", "unlock", "enable", "disable",
    "replace", "restore", "modify", "change", "toggle", "rerun", "cancel", "reply", "resolve",
    "accept", "reject", "import", "convert", "migrate", "sync",
];

/// Compound nouns read as one word (normalized form), so the verb inside them is not taken as the tool's verb
/// (`get_merge_request` is a read; `merge_merge_request` is not).
const COMPOUNDS: &[(&str, &str)] = &[("merge_request", "mergerequest")];

/// The one spelling every rule here reads: camelCase split into words at each lower-to-upper step (`GitHub` ->
/// `git_hub`, `AzureDevOps` -> `azure_dev_ops`), lower case, every run of characters that are not ASCII letters or
/// digits (`.`, `-`, `__`, spaces) turned into one `_`, no `_` at either end, then [`COMPOUNDS`] joined.
#[must_use]
pub fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    let mut prev_lower = false;
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            if c.is_ascii_uppercase() && prev_lower {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
            prev_lower = c.is_ascii_lowercase() || c.is_ascii_digit();
        } else {
            if !out.is_empty() && !out.ends_with('_') {
                out.push('_');
            }
            prev_lower = false;
        }
    }
    while out.ends_with('_') {
        out.pop();
    }
    for (compound, joined) in COMPOUNDS {
        out = out.replace(compound, joined);
    }
    out
}

fn words(text: &str) -> Vec<String> {
    normalize(text)
        .split('_')
        .filter(|w| !w.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The ONE forge test ([`is_forge`] and [`refuses_tool_name`] both use it): the word index right after the earliest
/// [`FORGE_NAMES`] entry found as whole words in `normalize(name)` (multi-word entries included), or `None`.
fn forge_end(name: &str) -> Option<usize> {
    let parts = words(name);
    FORGE_NAMES
        .iter()
        .filter_map(|forge| {
            let needle: Vec<&str> = forge.split('_').collect();
            parts
                .windows(needle.len())
                .position(|w| w.iter().map(String::as_str).eq(needle.iter().copied()))
                .map(|at| at + needle.len())
        })
        .min()
}

/// True when the connector alias (`github_app`, `GitLab-Work`, `azure_devops`, `AzureDevOps`) names a code forge.
#[must_use]
pub fn is_forge(alias: &str) -> bool {
    forge_end(alias).is_some()
}

/// True when `tool` (the remote tool name) only reads: its first word is a read verb or its last word a read suffix,
/// and no word is a write word. An unknown verb is not a read (fails closed).
#[must_use]
pub fn is_read_only(tool: &str) -> bool {
    let all = words(tool);
    let (Some(first), Some(last)) = (all.first(), all.last()) else {
        return false;
    };
    (READ_VERBS.contains(&first.as_str()) || READ_SUFFIXES.contains(&last.as_str()))
        && !all.iter().any(|w| WRITE_WORDS.contains(&w.as_str()))
}

/// The decision for one connector tool call, by alias and remote tool name: `true` = refuse.
#[must_use]
pub fn refuses(alias: &str, tool: &str) -> bool {
    is_forge(alias) && !is_read_only(tool)
}

/// The decision for a tool name as Vibe reports it (`connector_github_app.push_files`, or the legacy
/// `connector_github_app_push_files`; any separators or camelCase). `None` = not a connector tool; `Some(true)` =
/// refuse. A name with a forge anywhere in it is judged as a forge call, and it passes only when its tool part is
/// provably read-only (fails closed).
#[must_use]
pub fn refuses_tool_name(name: &str) -> Option<bool> {
    let parts = words(name);
    if parts.first().map(String::as_str) != Some("connector") || parts.len() < 2 {
        return None;
    }
    let Some(end) = forge_end(name) else {
        return Some(false);
    };
    // Unified form `connector_<alias>.<tool>`: the alias is Vibe-normalized (no dots), so the first dot of the raw
    // name starts the tool, which must read.
    if let Some((_, tool)) = name.split_once('.') {
        return Some(!is_read_only(tool));
    }
    // Legacy form (`_` joins alias and tool, split unknown): it reads when some split after the forge reads
    // (`app_get_commit` -> `get_commit`) and no word after the forge is a write word.
    let after = parts.get(end..).unwrap_or(&[]);
    let reads = (0..after.len()).any(|i| is_read_only(&after[i..].join("_")))
        && !after.iter().any(|w| WRITE_WORDS.contains(&w.as_str()));
    Some(!reads)
}
