//! The classification table: every client request method plus explicitly denied retired methods, exactly once.
//!
//! Depends on: vw-protocol (method markers for typed checks), class, error. Called by: gate
//! (dispatch), tests (`classified_methods`, `forwarded_without_unified_evidence`).
//! Invariants:
//! - One row per name in `vw_protocol::REQUEST_METHODS` (test `every_request_method_is_classified_
//!   exactly_once`), so a Vibe update that adds a method fails the test until it is classified.
//! - A method absent from the table is denied (`DenyReason::Unclassified`).
//! - Forwarded rows have unified-harness evidence in the protocol bench's report
//!   (`coverage[method].ok_on` contains `unified`) unless listed in
//!   [`FORWARDED_WITHOUT_UNIFIED_EVIDENCE`], which a test keeps exact.
//!
//! Classification rules (why a row sits where it does):
//! - Forwarded: reads and turn/session controls whose worst effect stays inside the user's own
//!   Vibe state and that the bench exercised on the unified harness. A forwarded row can still
//!   need a confirmation for some params (`session_rules`): `session/shellCommand` with any action
//!   but `interrupt` runs a host command with no approval in Vibe, so it needs a native
//!   confirmation bound to the exact command; `agentConfig` escalations on
//!   the session entry methods likewise.
//! - Privileged (P1): changes config or persistent state; the handler pins the shape.
//! - Confirmed (P2): installs or runs new code, adds a remote endpoint, changes where traffic
//!   goes, sends the repo or project data off the machine, or is destructive beyond one session.
//!   `projectLinks/*` and `vibeCode/*` are all P2, reads included.
//! - Denied: credentials, privacy-off telemetry, internal-only and dedicated-entry-point methods,
//!   legacy-only methods (we run unified), methods that answer `method_not_found`,
//!   and methods nobody exercised that move session state (`session/relocate`); worktree `prune` and
//!   `reap`, which remove dirty worktrees without naming their files.
//!
//! Aliases (`mcp/*` vs `mcp_catalog/*`) share a class; a P1 or P2 class needs no bench evidence
//! because it forwards nothing without a handler or a token.

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::LazyLock;
use vw_protocol::RequestMethod;
use vw_protocol::request;

use crate::class::{Class, P1};
use crate::error::DenyReason;

/// Validates params against the generated type of one method.
pub(crate) type Check = fn(&Value) -> Result<(), String>;

/// One table row.
pub(crate) struct Row {
    pub(crate) method: &'static str,
    pub(crate) class: Class,
    /// Typed validation for rows that forward (Forwarded and Confirmed); `None` for the rest.
    pub(crate) check: Option<Check>,
}

/// Typed check for method marker `M`: params must deserialize into `M::Params` and must not carry
/// top-level fields that type does not know (serde would silently drop them, but the original
/// value is what gets forwarded, so an unknown field would reach Vibe).
///
/// Two validation rules:
/// - When the type is an object, the params must be a JSON object. serde also builds a struct
///   from a JSON array (fields by position); that form skipped the unknown-field check and was
///   forwarded as an array to Vibe, whose handlers take a dict.
/// - The error text never quotes a params value (`error.rs` invariant). serde's messages do
///   (`invalid type: string "..."`, `unknown variant ...`), so only messages known to be
///   value-free (a missing field names our own key) are passed through.
pub(crate) fn typed<M>(params: &Value) -> Result<(), String>
where
    M: RequestMethod,
    M::Params: DeserializeOwned + Serialize,
{
    let parsed: M::Params = serde_json::from_value(params.clone()).map_err(|e| value_free(&e))?;
    let echoed =
        serde_json::to_value(&parsed).map_err(|_| "params cannot be re-encoded".to_owned())?;
    match (params.as_object(), echoed.as_object()) {
        (Some(given), Some(known)) => {
            if let Some(extra) = given.keys().find(|k| !known.contains_key(*k)) {
                return Err(format!("unknown field `{extra}`"));
            }
        }
        (None, Some(_)) => return Err("params must be an object".to_owned()),
        _ => {}
    }
    Ok(())
}

/// A description of a serde error that cannot contain a params value.
fn value_free(error: &serde_json::Error) -> String {
    let text = error.to_string();
    if text.starts_with("missing field `") {
        text
    } else {
        "a field has the wrong type or value".to_owned()
    }
}

macro_rules! fwd {
    ($m:literal, $t:ident) => {
        Row {
            method: $m,
            class: Class::Forwarded,
            check: Some(typed::<request::$t>),
        }
    };
}
macro_rules! p1 {
    ($m:literal, $k:ident) => {
        Row {
            method: $m,
            class: Class::Privileged(P1::$k),
            check: None,
        }
    };
}
macro_rules! p2 {
    ($m:literal, $t:ident) => {
        Row {
            method: $m,
            class: Class::Confirmed,
            check: Some(typed::<request::$t>),
        }
    };
}
/// A confirmed row whose check is the MCP add rule (`mcp_url`): the generated type plus the
/// address policy, so a refused address never reaches the native dialog.
macro_rules! p2_add {
    ($m:literal, $t:ident) => {
        Row {
            method: $m,
            class: Class::Confirmed,
            check: Some(crate::mcp_url::check_add::<request::$t>),
        }
    };
}
/// A confirmed row whose check is the proxy-settings rule (`proxy_write`): the generated type plus the value
/// rules, so a refused value never reaches the native dialog.
macro_rules! p2_proxy {
    ($m:literal, $t:ident) => {
        Row {
            method: $m,
            class: Class::Confirmed,
            check: Some(crate::proxy_write::check_write::<request::$t>),
        }
    };
}
macro_rules! deny {
    ($m:literal, $r:ident) => {
        Row {
            method: $m,
            class: Class::Denied(DenyReason::$r),
            check: None,
        }
    };
}

/// Forwarded methods without `unified` in the bench report's `ok_on`, each with the evidence that
/// justifies forwarding it anyway. A test fails if this list and the report disagree.
pub const FORWARDED_WITHOUT_UNIFIED_EVIDENCE: &[(&str, &str)] = &[
    (
        "session/rewind",
        "the bench's rewind runs: rewind in place and as a fork works on the unified harness when turn/start carries clientUserMessageId; the final report only has the earlier legacy-harness run",
    ),
    (
        "session/subagents/interrupt",
        "not covered by the bench; interrupts owned children through their parent runtime; real unified-harness child and sibling isolation is exercised by vw_l13m_tree_loops",
    ),
    (
        "session/turn/queue/steer",
        "not covered by the bench; sibling of the queue ops that pass on unified; params are three ids",
    ),
];

static TABLE: &[Row] = &[
    deny!("initialize", InternalOnly),
    deny!("imports/discover", NoUserInterface),
    deny!("imports/start", NoUserInterface),
    deny!("imports/read", NoUserInterface),
    deny!("session/turn/queue/edit/start", NoUserInterface),
    deny!("session/turn/queue/edit/save", NoUserInterface),
    deny!("session/turn/queue/edit/cancel", NoUserInterface),
    deny!("session/turn/queue/edit/abandon", NoUserInterface),
    // New in Vibe 3.0.1/3.0.2 (experimental upstream); no Vibe Work screen uses them in 0.1.2.
    // `connector_catalog/connect` makes a no-login connector usable on the user's Mistral account.
    deny!("connector_catalog/connect", NoUserInterface),
    fwd!("session/subagents/interrupt", SessionSubagentsInterrupt),
    deny!("session/subagents/read", NoUserInterface),
    deny!("session/subagents/wait", NoUserInterface),
    // New in Vibe 3.0.3 (session artifacts: pull and merge requests linked to the session's branch,
    // experimental upstream); no Vibe Work screen uses them in 0.1.2. `artifacts/sync` asks the
    // provider (`gh` / `glab`) for the current state.
    deny!("artifacts/list", NoUserInterface),
    deny!("artifacts/markViewed", NoUserInterface),
    deny!("artifacts/sync", NoUserInterface),
    fwd!("account/read", AccountRead),
    p2!("agents/install", AgentsInstall),
    deny!("agents/list", NotOnUnified),
    p2!("agents/uninstall", AgentsUninstall),
    deny!("callback/result", DedicatedEntryPoint),
    // New in Vibe 2.26.1. Read-only in Vibe's source (`app_server/_resources.py`: the effective config
    // of one session), but no bench scenario has exercised it on the unified harness and Vibe Work does not call it:
    // denied like `session/relocate` until a scenario covers it, then forwarded.
    deny!("config/effective/read", NoEvidence),
    fwd!("config/fields/read", ConfigFieldsRead),
    fwd!("config/proxy/read", ConfigProxyRead),
    p2_proxy!("config/proxy/write", ConfigProxyWrite),
    p1!("config/model/write", ModelWrite),
    fwd!("config/read", ConfigRead),
    fwd!("config/reload", ConfigReload),
    fwd!("config/schema", ConfigSchema),
    p1!("config/write", ConfigWrite),
    fwd!("connectors/auth/read", ConnectorsAuthRead),
    fwd!("connectors/read", ConnectorsRead),
    fwd!("connectors/refresh", ConnectorsRefresh),
    p2!(
        "connector_catalog/auth/request",
        ConnectorCatalogAuthRequest
    ),
    fwd!("connector_catalog/read", ConnectorCatalogRead),
    fwd!("connector_catalog/refresh", ConnectorCatalogRefresh),
    p1!("connector_catalog/toggle", ConnectorToggle),
    deny!("diagnostics/list", NotSupported),
    fwd!("diagnostics/logs/read", DiagnosticsLogsRead),
    fwd!("events/read", EventsRead),
    deny!("feedback/record", Privacy),
    fwd!("feedback/shouldShow", FeedbackShouldShow),
    deny!("session/history/get", NotOnUnified),
    fwd!("identity/read", IdentityRead),
    fwd!("loops/clear", LoopsClear),
    fwd!("loops/create", LoopsCreate),
    fwd!("loops/delete", LoopsDelete),
    fwd!("loops/list", LoopsList),
    p2_add!("mcp/add", McpAdd),
    p2!("mcp/login", McpLogin),
    p1!("mcp/logout", McpNamed),
    fwd!("mcp/read", McpRead),
    fwd!("mcp/refresh", McpRefresh),
    p1!("mcp/toggle", McpToggle),
    p2_add!("mcp_catalog/add", McpCatalogAdd),
    p2!("mcp_catalog/login", McpCatalogLogin),
    p1!("mcp_catalog/logout", McpNamed),
    fwd!("mcp_catalog/read", McpCatalogRead),
    fwd!("mcp_catalog/refresh", McpCatalogRefresh),
    p1!("mcp_catalog/remove", McpNamed),
    p1!("mcp_catalog/toggle", McpToggle),
    fwd!("narration/summarize", NarrationSummarize),
    fwd!("plugin/info", PluginInfo),
    // Only the core may reload: it holds the chat through the reopen and binds its host hooks again.
    deny!("plugin/reload", InternalOnly),
    fwd!("plugins/read", PluginsRead),
    fwd!("plugin_catalog/read", PluginCatalogRead),
    deny!("providerAuth/read", Credential),
    p2!("projectLinks/create", ProjectLinksCreate),
    p2!("projectLinks/inspectRoot", ProjectLinksInspectRoot),
    p2!("projectLinks/link", ProjectLinksLink),
    p2!("projectLinks/list", ProjectLinksList),
    p2!("projectLinks/picker/load", ProjectLinksPickerLoad),
    p2!("projectLinks/picker/loadMore", ProjectLinksPickerLoadMore),
    p2!("projectLinks/resolveRoot", ProjectLinksResolveRoot),
    p2!("projectLinks/save", ProjectLinksSave),
    p2!("projectLinks/unlink", ProjectLinksUnlink),
    deny!("review/approve", NotOnUnified),
    deny!("review/baseline", NotOnUnified),
    deny!("review/hunks", NotOnUnified),
    deny!("review/revert", NotOnUnified),
    deny!("review/state", NotOnUnified),
    deny!("review/turnDiff", NotOnUnified),
    fwd!("runtime/read", RuntimeRead),
    p1!("session/agent/update", AgentUpdate),
    fwd!(
        "session/backgroundProcess/output",
        SessionBackgroundProcessOutput
    ),
    fwd!(
        "session/backgroundProcess/stop",
        SessionBackgroundProcessStop
    ),
    fwd!("session/compact", SessionCompact),
    fwd!("session/continue", SessionContinue),
    fwd!("session/context/inject", SessionContextInject),
    fwd!("session/delete", SessionDelete),
    fwd!("session/fork", SessionFork),
    fwd!("session/history/clear", SessionHistoryClear),
    fwd!("session/history/list", SessionHistoryList),
    // New in Vibe 2.26.1: streams which client holds each session's lease
    // (`session/lease/updated|ended` notifications, `app_server/_session_lease_subscription.py`). Read-only, never
    // exercised by the bench, unused by Vibe Work: denied until a scenario covers it (same rule as above).
    deny!("session/lease/subscribe", NoEvidence),
    fwd!("session/list", SessionList),
    fwd!("session/log/read", SessionLogRead),
    fwd!("session/markAsSeen", SessionMarkAsSeen),
    fwd!("session/pin", SessionPin),
    fwd!("session/archive", SessionArchive),
    fwd!("session/read", SessionRead),
    fwd!("session/ready/read", SessionReadyRead),
    fwd!("session/ready/wait", SessionReadyWait),
    deny!("session/relocate", NoEvidence),
    fwd!("session/rename", SessionRename),
    fwd!("session/resume", SessionResume),
    fwd!("session/rewind", SessionRewind),
    fwd!("session/rewind/read", SessionRewindRead),
    fwd!("session/settings/update", SessionSettingsUpdate),
    fwd!("session/shellCommand", SessionShellCommand),
    fwd!("session/start", SessionStart),
    fwd!("session/stop", SessionStop),
    deny!("session/title/update", NotSupported),
    fwd!("session/turns/list", SessionTurnsList),
    fwd!("setup/status", SetupStatus),
    deny!("setup/store-credential", Credential),
    p1!("setup/submit-choices", SetupChoices),
    deny!("shell/interrupt", NotSupported),
    deny!("shell/run", NotSupported),
    fwd!("skills/catalog", SkillsCatalog),
    p2!("skills/convertLocal", SkillsConvertLocal),
    fwd!("skills/detail", SkillsDetail),
    p2!("skills/import", SkillsImport),
    fwd!("skills/installed", SkillsInstalled),
    fwd!("skills/list", SkillsList),
    p1!("skills/remove", SkillRemove),
    p1!("skills/setAlias", SkillSetAlias),
    p1!("skills/setEnabled", SkillSetEnabled),
    p2!("skills/setLatest", SkillsSetLatest),
    p2!("skills/setVersion", SkillsSetVersion),
    fwd!("skills/updates", SkillsUpdates),
    fwd!("skills/versions", SkillsVersions),
    deny!("stats/read", NotSupported),
    deny!("telemetry/record", Privacy),
    // Not forwarded; the core answers it from `runtime/read` (vw-core-api `tools_list`).
    deny!("tools/list", NotSupported),
    fwd!("session/turn/enqueue", SessionTurnEnqueue),
    fwd!("session/turn/queue/read", SessionTurnQueueRead),
    fwd!("session/turn/queue/remove", SessionTurnQueueRemove),
    fwd!("session/turn/queue/replace", SessionTurnQueueReplace),
    fwd!("session/turn/queue/steer", SessionTurnQueueSteer),
    fwd!("session/turn/queue/resume", SessionTurnQueueResume),
    fwd!("turn/interrupt", TurnInterrupt),
    fwd!("turn/start", TurnStart),
    fwd!("turn/steer", TurnSteer),
    p2!("vibeCode/projects/cancel", VibeCodeProjectsCancel),
    p2!("vibeCode/projects/create", VibeCodeProjectsCreate),
    p2!("vibeCode/projects/loadMore", VibeCodeProjectsLoadMore),
    p2!("vibeCode/projects/open", VibeCodeProjectsOpen),
    p2!("vibeCode/projects/recover", VibeCodeProjectsRecover),
    p2!("vibeCode/projects/select", VibeCodeProjectsSelect),
    p2!("vibeCode/projects/unlink", VibeCodeProjectsUnlink),
    p2!("vibeCode/teleport/cancel", VibeCodeTeleportCancel),
    p2!(
        "vibeCode/teleport/push/respond",
        VibeCodeTeleportPushRespond
    ),
    p2!("vibeCode/teleport/start", VibeCodeTeleportStart),
    fwd!("workspace/git/checkouts", WorkspaceGitCheckouts),
    p1!("workspace/git/worktrees/limit/update", WorktreeLimit),
    fwd!("workspace/git/worktrees/list", WorkspaceGitWorktreesList),
    // Vibe Work never prunes worktrees by a count.
    deny!("workspace/git/worktrees/prune", UnnamedRemoval),
    deny!("workspace/git/worktrees/reap", UnnamedRemoval),
    deny!("workspace/git/worktrees/reap/cancel", NotSupported),
    p1!("workspace/git/worktrees/remove", WorktreeRemove),
    fwd!("workspace/prompt/prepare", WorkspacePromptPrepare),
    p1!("workspace/trust/decision", WorkspaceTrust),
    fwd!(
        "workspace/trust/untrustedConfig",
        WorkspaceTrustUntrustedConfig
    ),
    fwd!("workspace/trust/status", WorkspaceTrustStatus),
];

static INDEX: LazyLock<HashMap<&'static str, &'static Row>> =
    LazyLock::new(|| TABLE.iter().map(|row| (row.method, row)).collect());

/// Looks up a method; `None` means unclassified (the gate denies it).
pub(crate) fn row(method: &str) -> Option<&'static Row> {
    INDEX.get(method).copied()
}

/// The class of a client request method, or `None` when the method is not in the table.
#[must_use]
pub fn classify(method: &str) -> Option<Class> {
    row(method).map(|r| r.class.clone())
}

/// Every classified method with its class, in table order (tests walk this).
pub fn classified_methods() -> impl Iterator<Item = (&'static str, Class)> {
    TABLE.iter().map(|r| (r.method, r.class.clone()))
}

/// Forwarded methods that lack unified-harness evidence in the bench report, with the reason each
/// is forwarded anyway.
#[must_use]
pub fn forwarded_without_unified_evidence() -> &'static [(&'static str, &'static str)] {
    FORWARDED_WITHOUT_UNIFIED_EVIDENCE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_reload_can_only_run_inside_the_core_refresh() {
        assert_eq!(
            classify("plugin/reload"),
            Some(Class::Denied(DenyReason::InternalOnly))
        );
    }
}
