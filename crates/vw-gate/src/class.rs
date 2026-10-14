//! The four method classes of the gate and the typed-handler identifiers.
//!
//! Depends on: error (`DenyReason`). Called by: table (rows), gate (dispatch), tests.
//! Invariant: a method has exactly one class; no class means denied.

use crate::error::DenyReason;

/// How the gate treats one client request method.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Class {
    /// Reads and turn controls that worked on the unified harness: forwarded after typed
    /// validation (plus the session rules in `session_rules`).
    Forwarded,
    /// Changes config or persistent state inside a narrow typed shape, checked by a handler.
    Privileged(P1),
    /// Installs or runs new code, adds a remote endpoint, or sends the repo off the machine:
    /// needs a single-use token from a native confirmation.
    Confirmed,
    /// Never forwarded.
    Denied(DenyReason),
}

/// The typed handler of a privileged (P1) method. Several methods can share a handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum P1 {
    /// `config/write`: explicit key allowlist, value-typed.
    ConfigWrite,
    /// `config/model/write`: a configured model alias and/or a reasoning level.
    ModelWrite,
    /// `session/agent/update`: one of Vibe's five profiles.
    AgentUpdate,
    /// `mcp/toggle`, `mcp_catalog/toggle`: enable or disable a server, connector or tool.
    McpToggle,
    /// `connector_catalog/toggle`.
    ConnectorToggle,
    /// `mcp/logout`, `mcp_catalog/logout`, `mcp_catalog/remove`: act on one named server.
    McpNamed,
    /// `workspace/trust/decision`: trust only folders the user picked natively.
    WorkspaceTrust,
    /// `skills/setEnabled`.
    SkillSetEnabled,
    /// `skills/setAlias`.
    SkillSetAlias,
    /// `skills/remove`.
    SkillRemove,
    /// `setup/submit-choices`: theme only (provider and base URLs would redirect the API key).
    SetupChoices,
    /// `workspace/git/worktrees/limit/update`.
    WorktreeLimit,
    /// `workspace/git/worktrees/remove`: inspection is free, removal needs a confirmation.
    WorktreeRemove,
}
