//! Dispatch of the privileged (P1) methods to their typed handlers, plus the small shape-only
//! handlers (toggles, skills, setup choices, worktrees, trust, profile switch).
//!
//! Depends on: `config_rules`, shape, profile, picked, error, class. Called by: gate.
//! Invariants:
//! - A handler never changes the params: valid params are forwarded byte-exact.
//! - A handler returns `Ok(Some(summary))` when the call is valid but escalating, which makes the
//!   gate demand a native confirmation token; `Ok(None)` when it may go straight through.
//! - Anything outside the shape is an error and nothing is sent.

use std::collections::HashSet;

use serde_json::Value;

use crate::class::P1;
use crate::config_rules::{config_write, model_write};
use crate::error::{DenyReason, GateError};
use crate::picked::{PickedFolders, repo_root};
use crate::profile::{PROFILES, escalates};
use crate::shape::{Field, Kind, check, opt, req};

/// What the handlers may consult.
pub(crate) struct Ctx<'a> {
    /// Model aliases Vibe reported (`runtime/read`).
    pub(crate) aliases: &'a HashSet<String>,
    /// Folders the user picked natively.
    pub(crate) picked: &'a PickedFolders,
}

const ID: Kind = Kind::Text(128);
const NAME: Kind = Kind::Name(128);

const AGENT_UPDATE: &[Field] = &[
    req("sessionId", ID),
    req("agentName", Kind::OneOf(&PROFILES)),
];
const MCP_TOGGLE: &[Field] = &[
    opt("sessionId", ID),
    req("name", NAME),
    req("source", Kind::OneOf(&["connector", "server"])),
    req("disabled", Kind::Bool),
    opt("toolName", NAME),
];
const CONNECTOR_TOGGLE: &[Field] = &[
    req("alias", NAME),
    req("disabled", Kind::Bool),
    opt("toolName", NAME),
    opt("sessionId", ID),
];
const MCP_NAMED: &[Field] = &[opt("sessionId", ID), req("name", NAME)];
const SKILL_SET_ENABLED: &[Field] = &[
    req("sessionId", ID),
    req("name", NAME),
    req("enabled", Kind::Bool),
];
const SKILL_SET_ALIAS: &[Field] = &[
    req("sessionId", ID),
    req("name", NAME),
    req("alias", NAME),
    opt("scope", NAME),
];
const SKILL_REMOVE: &[Field] = &[req("sessionId", ID), req("name", NAME), opt("scope", NAME)];
/// Theme only: the provider and base-URL fields of `setup/submit-choices` could point the API key
/// at another host.
const SETUP_CHOICES: &[Field] = &[req("theme", Kind::Text(64))];
const WORKTREE_LIMIT: &[Field] = &[req("limit", Kind::Int(1, 64))];
const WORKTREE_REMOVE: &[Field] = &[
    req("cwd", Kind::Text(4096)),
    opt("force", Kind::Bool),
    opt("deleteBranch", Kind::Bool),
    opt("inspect", Kind::Bool),
];
const TRUST: &[Field] = &[
    req(
        "decision",
        Kind::OneOf(&["decline", "trust_cwd", "trust_repo"]),
    ),
    opt("cwd", Kind::Text(4096)),
    opt("sessionId", ID),
];

fn policy(method: &str, rule: &str) -> GateError {
    GateError::Denied {
        method: method.to_owned(),
        reason: DenyReason::Policy(rule.to_owned()),
    }
}

/// Runs the handler of `kind`. See the module docs for the return value.
pub(crate) fn check_p1(
    kind: P1,
    method: &str,
    params: &Value,
    ctx: &Ctx<'_>,
) -> Result<Option<String>, GateError> {
    match kind {
        P1::ConfigWrite => config_write(method, params, ctx.aliases),
        P1::ModelWrite => model_write(method, params, ctx.aliases).map(|()| None),
        P1::AgentUpdate => {
            let object = check(method, params, AGENT_UPDATE)?;
            let escalating = object
                .get("agentName")
                .and_then(Value::as_str)
                .is_some_and(escalates);
            Ok(escalating.then(|| {
                format!(
                    "Switch this session to {}: the agent will run every tool without asking",
                    vw_vocabulary::label("accessModes", "auto-approve")
                )
            }))
        }
        P1::McpToggle => shape_only(method, params, MCP_TOGGLE),
        P1::ConnectorToggle => shape_only(method, params, CONNECTOR_TOGGLE),
        P1::McpNamed => shape_only(method, params, MCP_NAMED),
        P1::SkillSetEnabled => shape_only(method, params, SKILL_SET_ENABLED),
        P1::SkillSetAlias => shape_only(method, params, SKILL_SET_ALIAS),
        P1::SkillRemove => shape_only(method, params, SKILL_REMOVE),
        P1::SetupChoices => shape_only(method, params, SETUP_CHOICES),
        P1::WorktreeLimit => shape_only(method, params, WORKTREE_LIMIT),
        P1::WorktreeRemove => worktree_remove(method, params),
        P1::WorkspaceTrust => workspace_trust(method, params, ctx.picked),
    }
}

fn shape_only(method: &str, params: &Value, fields: &[Field]) -> Result<Option<String>, GateError> {
    check(method, params, fields).map(|_| None)
}

/// Inspecting a worktree (`inspect: true`) is free. Anything else removes it, and must state `force`,
/// `deleteBranch` and `inspect: false` explicitly (with Vibe's defaults a dirty
/// worktree was removed and its branch deleted, its work only in a hidden `refs/vibe/reaped` ref); with
/// them it needs a native confirmation. The core's `worktrees remove` builds the dialog text with the files
/// (`vw-core-api` `worktrees_remove.rs`); any other caller gets the dialog built from the params.
fn worktree_remove(method: &str, params: &Value) -> Result<Option<String>, GateError> {
    let object = check(method, params, WORKTREE_REMOVE)?;
    let flag = |key: &str| object.get(key).and_then(Value::as_bool);
    if flag("inspect") == Some(true) && flag("force") != Some(true) {
        return Ok(None);
    }
    if flag("inspect") != Some(false) || flag("force").is_none() || flag("deleteBranch").is_none() {
        return Err(policy(
            method,
            "a worktree removal must state force, deleteBranch and inspect: false explicitly (never Vibe's defaults)",
        ));
    }
    Ok(Some(
        "Remove a git worktree (the folder and possibly its branch are deleted)".to_owned(),
    ))
}

/// Declining is always safe. Trusting a folder lets Vibe run project hooks and servers from it, so
/// it works only for a folder the user picked natively (or inside one).
///
/// `trust_repo` does not trust `cwd`: Vibe writes the repository root above it to the persistent
/// trust store (`apply_workspace_trust_decision` uses `prompt.repo_root`). That root is the folder
/// that must be inside a picked folder, so the gate checks the path Vibe uses rather than the one
/// it was given. Checking only `cwd` would let the dialog name a different path from the one trusted.
/// It still needs a confirmation because it widens trust beyond `cwd`.
///
/// Remaining window (documented in `picked`): the check reads the filesystem now and Vibe reads it
/// again later, so a local process that can rewrite the picked folder could move the `.git` marker
/// in between.
fn workspace_trust(
    method: &str,
    params: &Value,
    picked: &PickedFolders,
) -> Result<Option<String>, GateError> {
    let object = check(method, params, TRUST)?;
    let decision = object
        .get("decision")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if decision == "decline" {
        return Ok(None);
    }
    let cwd = object.get("cwd").and_then(Value::as_str);
    let Some(cwd) = cwd.filter(|c| picked.contains(c)) else {
        return Err(policy(method, "folder was not picked by the user"));
    };
    if decision == "trust_repo" && !repo_root(cwd).is_some_and(|root| picked.contains_path(&root)) {
        return Err(policy(
            method,
            "the repository root is not inside a folder the user picked",
        ));
    }
    Ok((decision == "trust_repo").then(|| {
        "Trust the whole git repository of this folder (project hooks and servers may run)"
            .to_owned()
    }))
}
