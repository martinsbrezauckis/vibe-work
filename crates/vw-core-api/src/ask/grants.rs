//! Session grants for Vibe's approvals, kept on Vibe Work's side.
//!
//! Every Yes on a Vibe approval is sent to Vibe as `approve` (once), so Vibe stores nothing: neither its
//! in-memory session rule nor, for "Always allow", a lasting rule in Vibe Work's own `config.toml`. "Allow for
//! this session" also keeps a [`SessionGrant`] here, and a later approval in the same session whose tool and
//! required-permission set equal a kept grant is answered `approve` by the core (the pump), recorded
//! `Allowed { Session, Grant }`.
//!
//! What can be kept (`scope_of`, from the callback Vibe sent; source: pinned Vibe 2.26.1):
//! - a call with required permissions: that tool + exactly those permissions (`vw_ask::GrantPermission`);
//! - a call without them for a tool Vibe grants whole (`_unified_permissions.py:545-560`, MCP/provided tools and
//!   the file tools): every call of the tool in this session (`every_call`, the card says so before the click);
//! - a call without them for a builtin Vibe cannot scope (`grant()` returns early, `_unified_permissions.py:563`:
//!   `_FALLBACK_SCOPES` and `_PROCESS_UNGATED_BUILTINS`): nothing, so no session button is offered.
//!
//! - a call of one of Vibe Work's own tools that runs a command (`mcp_vw.process_start`, `mcp_vw.secret_run`): only
//!   that exact command in that working folder, so a grant covers only what the user saw.
//!   Vibe would grant the whole tool (every background command); the key is
//!   the call's own arguments instead, compared as exact text. `mcp_vw.process_restart` names only a pid, whose
//!   command the card does not show: no session answer.
//!
//! A malformed permission (missing or non-text field, blank pattern) keeps nothing either: fail closed.
//!
//! Lifetime: in memory only (0.1.2). A grant ends when its session (or the parent session of a subagent's card)
//! is deleted or archived, with Remove, or when Vibe Work quits. It follows a hand-off (compaction gives the
//! chat a new session id). It survives a Vibe server restart and an idle channel close: the chat is the same.
//!
//! Depends on: `vw_ask` (grant types). Called by: `ask::cards` (offers, the user's answer), `pump` (matching,
//! session end, hand-off), `session_end`, `ask::commands` (list, remove).
use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use serde_json::{Value, json};
use vw_ask::{AnsweredBy, CoversPart, GrantOffer, GrantPermission, SessionGrant};

/// Builtins whose permission-less call Vibe answers without remembering anything (see the module docs), plus
/// the shell tools' own names in Vibe's older approval path (`_turns.py`), so they never get a tool-wide grant.
const UNSCOPED_BUILTINS: &[&str] = &[
    "file_system.bash",
    "process.start",
    "process.write",
    "process.output",
    "process.list",
    "process.stop",
    "bash",
    "powershell",
    "git_bash",
];

/// Vibe Work's own MCP tools as Vibe routes them (the core's `vw` server).
const OWN_TOOLS: &str = "mcp_vw.";

/// The scope name of the key a grant on one of our own command-running tools keeps (not one of Vibe's scopes).
const OWN_CALL_SCOPE: &str = "vibe_work_call";

/// How one of Vibe Work's own tools may be granted for the session (see the module docs).
enum Own {
    /// Not one of our command-running tools: Vibe's rules apply.
    Other,
    /// No session answer: nothing on the card names what would run.
    Nothing,
    /// Only calls with exactly this key (and its label for the list, and what it covers in words).
    Key(GrantPermission, String, Vec<CoversPart>),
}

fn own_call(tool: &str, input: &Value) -> Own {
    use vw_agent_tools::wire::tools as names;
    let Some(name) = tool.strip_prefix(OWN_TOOLS) else {
        return Own::Other;
    };
    let text = |key: &str| {
        input
            .get(key)
            .and_then(Value::as_str)
            .filter(|t| !t.trim().is_empty())
    };
    let field = |key: &str| input.get(key).cloned().unwrap_or(Value::Null);
    // Exact text, never collapsed or parsed: narrower than any rule Vibe or a shell would apply.
    let key = |value: Value, label: String, covers: Vec<CoversPart>| {
        Own::Key(
            GrantPermission {
                scope: OWN_CALL_SCOPE.to_owned(),
                session_pattern: value.to_string(),
                exact: true,
            },
            label,
            covers,
        )
    };
    // " in `{folder}`" when the call names its working folder (the key keeps it, so the words say it).
    let in_folder = |lead: &str| {
        text("cwd").map_or_else(Vec::new, |cwd| {
            vec![CoversPart::words(lead), CoversPart::code(cwd)]
        })
    };
    match name {
        names::PROCESS_START => text("command").map_or(Own::Nothing, |command| {
            key(
                json!({"command": command, "cwd": field("cwd")}),
                command.to_owned(),
                [
                    vec![CoversPart::words("only "), CoversPart::code(command)],
                    in_folder(" in "),
                ]
                .concat(),
            )
        }),
        // The secret bindings are part of what runs; `purpose` and the timeout are not.
        names::SECRET_RUN => text("program").map_or(Own::Nothing, |program| {
            let label = std::iter::once(program.to_owned())
                .chain(
                    input["args"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|a| a.as_str().map(str::to_owned)),
                )
                .collect::<Vec<_>>()
                .join(" ");
            let covers = [
                vec![
                    CoversPart::words("only "),
                    CoversPart::code(&label),
                    CoversPart::words(" with the same secrets"),
                ],
                in_folder(", in "),
            ]
            .concat();
            key(
                json!({"program": program, "args": field("args"), "cwd": field("cwd"),
                    "env": field("env")}),
                label,
                covers,
            )
        }),
        names::PROCESS_RESTART => Own::Nothing,
        _ => Own::Other,
    }
}

/// What "Allow for this session" may keep for one approval callback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Scopeable {
    /// Nothing can honestly be kept: the card offers no session answer.
    No,
    /// A grant for `tool` and `permissions` (sorted, without duplicates); empty means every call of the tool.
    Grant {
        tool: String,
        permissions: Vec<GrantPermission>,
        labels: Vec<String>,
        /// What the grant covers, in words (the card's covers line).
        covers: Vec<CoversPart>,
    },
}

impl Scopeable {
    /// A grant can be kept.
    pub(crate) fn keeps(&self) -> bool {
        matches!(self, Self::Grant { .. })
    }
    /// The grant would cover every call of the tool.
    pub(crate) fn every_call(&self) -> bool {
        matches!(self, Self::Grant { permissions, .. } if permissions.is_empty())
    }
}

/// Joins neighbouring pieces of the same kind, so equal sentences have equal pieces.
fn merged(parts: Vec<CoversPart>) -> Vec<CoversPart> {
    let mut out: Vec<CoversPart> = Vec::with_capacity(parts.len());
    for part in parts {
        match out.last_mut() {
            Some(last) if last.code == part.code => last.text.push_str(&part.text),
            _ => out.push(part),
        }
    }
    out
}

/// What one of Vibe's required permissions lets a grant answer, in words. Built from the
/// permission the matcher compares, so the words never promise more than equality matches: a glob is "… that
/// match `{pattern}`", the call's own text (`exact`) is "only the exact …" naming it, an outside-folder path is that
/// path. The grant list and the Allowed page show the same words without the card, so they always name the text.
fn permission_words(permission: &GrantPermission) -> Vec<CoversPart> {
    const EXACT_PATH: &str = "vibe-path:exact:";
    let pattern = permission.session_pattern.as_str();
    let only = |what: &str| vec![CoversPart::words(what), CoversPart::code(pattern)];
    let matching = |what: &str| vec![CoversPart::words(what), CoversPart::code(pattern)];
    match permission.scope.as_str() {
        "command_pattern" if permission.exact => only("only the exact command "),
        "command_pattern" => matching("commands that match "),
        "outside_directory" => pattern.strip_prefix(EXACT_PATH).map_or_else(
            || {
                if permission.exact {
                    only("only the path ")
                } else {
                    matching("paths that match ")
                }
            },
            |path| vec![CoversPart::words("only the path "), CoversPart::code(path)],
        ),
        "file_pattern" if permission.exact => only("only the file "),
        "file_pattern" => matching("files that match "),
        "url_pattern" if permission.exact => only("only the address "),
        "url_pattern" => matching("addresses that match "),
        _ if permission.exact => only("only "),
        _ => matching("calls that match "),
    }
}

/// The whole built-in action, with no folder or input restriction implied. Unknown tools retain their exact name.
fn whole_tool_words(tool: &str) -> Vec<CoversPart> {
    let action = match tool {
        "file_system.write_file" | "write_file" => "creating and changing files anywhere",
        "file_system.edit" | "file_system.search_replace" | "edit" => "changing files anywhere",
        "file_system.read_file" | "read_file" => "reading files anywhere",
        "file_system.grep" | "grep" => "searching file contents anywhere",
        "file_system.glob" | "glob" => "finding files anywhere",
        "file_system.list_directory" | "list_directory" => "listing folders anywhere",
        "web_fetch" | "web.fetch" => "reading web pages",
        "web_search" | "web.search" => "searching the web",
        "task" => "delegating work to subagents",
        "skill" | "skill.read" => "reading skill instructions",
        "self.sleep" | "self.wait" | "wait" => "waiting",
        "agents.spawn" | "subagent.spawn" => "starting subagents",
        "agents.list_configs" => "listing agent configurations",
        "agents.list" | "subagent.list" => "listing subagents",
        "subagent.wait" => "waiting for subagents",
        "agents.send_message" | "subagent.send_message" => "sending messages to subagents",
        "agents.interrupt" | "subagent.interrupt" => "interrupting subagents",
        "subagent.stop" => "stopping subagents",
        "todo" => "updating the task list",
        "ask_user_question" => "asking you questions",
        "exit_plan_mode" => "finishing a plan",
        _ => {
            return vec![
                CoversPart::words("every call of "),
                CoversPart::code(tool),
                CoversPart::words(" in this session"),
            ];
        }
    };
    vec![CoversPart::words(&format!("{action} in this session"))]
}

/// What a session grant would keep for the approval callback `entry` (Vibe's `ApprovalCallbackDetail`).
pub(crate) fn scope_of(entry: &Value) -> Scopeable {
    let detail = &entry["detail"];
    let Some(tool) = detail["effect"]["toolName"]
        .as_str()
        .filter(|t| !t.is_empty())
    else {
        return Scopeable::No;
    };
    if detail["kind"] != "approval" {
        return Scopeable::No;
    }
    // Vibe's model defaults the list to empty (`models.py:327`).
    let raw = match &detail["requiredPermissions"] {
        Value::Null => Vec::new(),
        Value::Array(items) => items.clone(),
        _ => return Scopeable::No,
    };
    let mut permissions = Vec::with_capacity(raw.len());
    let mut labels = Vec::with_capacity(raw.len());
    for item in &raw {
        let text = |key: &str| item[key].as_str();
        let (Some(scope), Some(invocation), Some(pattern)) = (
            text("scope"),
            text("invocationPattern"),
            text("sessionPattern"),
        ) else {
            return Scopeable::No;
        };
        if scope.is_empty() || pattern.trim().is_empty() {
            return Scopeable::No;
        }
        permissions.push(GrantPermission::from_wire(scope, invocation, pattern));
        labels.push(text("label").unwrap_or(pattern).to_owned());
    }
    let mut phrases: Vec<Vec<CoversPart>> = permissions.iter().map(permission_words).collect();
    match own_call(tool, &detail["effect"]["input"]) {
        Own::Nothing => return Scopeable::No,
        Own::Key(permission, label, words) => {
            permissions.push(permission);
            labels.push(label);
            phrases.push(words);
        }
        Own::Other => {}
    }
    if permissions.is_empty() && UNSCOPED_BUILTINS.contains(&tool) {
        return Scopeable::No;
    }
    permissions.sort();
    permissions.dedup();
    labels.sort();
    labels.dedup();
    let covers = if phrases.is_empty() {
        whole_tool_words(tool)
    } else {
        merged(phrases.join(&CoversPart::words(" and ")))
    };
    Scopeable::Grant {
        tool: tool.to_owned(),
        permissions,
        labels,
        covers,
    }
}

/// An open approval's grant candidate, kept until it is answered or closes. The answer path takes it out before
/// the answer is sent ([`Grants::claim`]): Vibe may close the callback (and the pump forget it) before the send
/// returns.
#[derive(Debug, Clone)]
pub(crate) struct Pending {
    session: String,
    owner: String,
    scope: Scopeable,
}

struct Kept {
    grant: SessionGrant,
    /// The session whose end ends the grant: the card's session, or its parent for a subagent's card.
    owner: String,
}

#[derive(Default)]
struct State {
    next: u64,
    kept: Vec<Kept>,
    pending: BTreeMap<String, Pending>,
}

/// The run's session grants (one per core).
#[derive(Default)]
pub(crate) struct Grants {
    state: Mutex<State>,
}

impl Grants {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Remembers what "Allow for this session" would keep for the open approval `callback_id`.
    pub(crate) fn note_callback(
        &self,
        callback_id: &str,
        session: &str,
        owner: &str,
        scope: Scopeable,
    ) {
        self.lock().pending.insert(
            callback_id.to_owned(),
            Pending {
                session: session.to_owned(),
                owner: owner.to_owned(),
                scope,
            },
        );
    }

    /// The approval was answered or closed: its candidate is no longer needed.
    pub(crate) fn forget_callback(&self, callback_id: &str) {
        self.lock().pending.remove(callback_id);
    }

    /// Takes the grant candidate of `callback_id` before its answer is sent.
    pub(crate) fn claim(&self, callback_id: &str) -> Option<Pending> {
        self.lock().pending.remove(callback_id)
    }

    /// Keeps the grant of a claimed candidate once Vibe took the `approve` (the user chose "Allow for this
    /// session"). An equal grant already kept is returned instead of a second copy. `None` when none can be kept.
    pub(crate) fn keep(&self, pending: Pending, by: AnsweredBy, now: u64) -> Option<SessionGrant> {
        let mut state = self.lock();
        let Scopeable::Grant {
            tool,
            permissions,
            labels,
            covers,
        } = pending.scope
        else {
            return None;
        };
        if let Some(kept) = state.kept.iter().find(|k| {
            k.grant.session == pending.session
                && k.grant.tool == tool
                && k.grant.permissions == permissions
        }) {
            return Some(kept.grant.clone());
        }
        state.next += 1;
        let grant = SessionGrant {
            id: format!("grant-{}", state.next),
            session: pending.session,
            tool,
            every_call: permissions.is_empty(),
            permissions,
            labels,
            created_at: now,
            by,
            covers,
            uses: 0,
            last_used_at: None,
        };
        state.kept.push(Kept {
            grant: grant.clone(),
            owner: pending.owner,
        });
        Some(grant)
    }

    /// The kept grant that answers the approval callback `entry` of `session`, if one does.
    ///
    /// A literal scope (`exact`) is answered only when its invocation is the pattern text itself: Vibe matches a
    /// literal as `kept pattern == invocation` (`core/tools/permissions.py:43`), while the pattern is the
    /// invocation's tokens joined by one space, so a whitespace variant (`'a  b'` for `'a b'`, another argument
    /// inside quotes) has the same pattern but would be asked again by Vibe. Asked here too.
    pub(crate) fn matching(&self, session: &str, entry: &Value) -> Option<SessionGrant> {
        if literal_differs_from_its_invocation(entry) {
            return None;
        }
        let Scopeable::Grant {
            tool, permissions, ..
        } = scope_of(entry)
        else {
            return None;
        };
        self.lock()
            .kept
            .iter()
            .find(|k| k.grant.covers(session, &tool, &permissions))
            .map(|k| k.grant.clone())
    }

    /// The kept "Allow for this session" of a Chrome Control key (`forms@shop.example`, a group with no site, or
    /// `closeTabs`) in `session`. Keyed by what the user saw on the card, not by Vibe's tool name.
    pub(crate) fn chrome_grant(&self, session: &str, key: &str) -> Option<SessionGrant> {
        let tool = chrome_tool(key);
        self.lock()
            .kept
            .iter()
            .find(|k| k.grant.session == session && k.grant.tool == tool)
            .map(|k| k.grant.clone())
    }

    /// Keeps a Chrome Control "Allow for this session" for `key` (a group on a site; an equal one already kept is
    /// returned). `label` names it in the grants list.
    pub(crate) fn keep_chrome(
        &self,
        session: &str,
        key: &str,
        label: &str,
        covers: Vec<CoversPart>,
        by: AnsweredBy,
        now: u64,
    ) -> SessionGrant {
        let tool = chrome_tool(key);
        let mut state = self.lock();
        if let Some(kept) = state
            .kept
            .iter()
            .find(|k| k.grant.session == session && k.grant.tool == tool)
        {
            return kept.grant.clone();
        }
        state.next += 1;
        let grant = SessionGrant {
            id: format!("grant-{}", state.next),
            session: session.to_owned(),
            tool,
            permissions: Vec::new(),
            labels: vec![label.to_owned()],
            every_call: true,
            created_at: now,
            by,
            covers,
            uses: 0,
            last_used_at: None,
        };
        state.kept.push(Kept {
            grant: grant.clone(),
            owner: session.to_owned(),
        });
        grant
    }

    /// Ends every grant whose tool `gone` names (a removed connector's tools). True when one ended.
    pub(crate) fn end_tools(&self, gone: impl Fn(&str) -> bool) -> bool {
        let mut state = self.lock();
        let before = state.kept.len();
        state.kept.retain(|k| !gone(&k.grant.tool));
        state.kept.len() != before
    }

    /// Every kept grant, oldest first.
    pub(crate) fn list(&self) -> Vec<SessionGrant> {
        self.lock().kept.iter().map(|k| k.grant.clone()).collect()
    }

    /// What "Allow for this session" would keep on each open card that offers it (the cards' covers lines).
    pub(crate) fn offers(&self) -> Vec<GrantOffer> {
        self.lock()
            .pending
            .iter()
            .filter_map(|(callback_id, p)| match &p.scope {
                Scopeable::Grant { covers, .. } => Some(GrantOffer {
                    callback_id: callback_id.clone(),
                    session: p.session.clone(),
                    covers: covers.clone(),
                }),
                Scopeable::No => None,
            })
            .collect()
    }

    /// Counts one call that grant `id` answered at `now` (the Allowed page's "used n times, last …"). False
    /// when the grant is gone (removed meanwhile).
    pub(crate) fn record_use(&self, id: &str, now: u64) -> bool {
        let mut state = self.lock();
        let Some(kept) = state.kept.iter_mut().find(|k| k.grant.id == id) else {
            return false;
        };
        kept.grant.uses = kept.grant.uses.saturating_add(1);
        kept.grant.last_used_at = Some(now);
        true
    }

    /// Removes one grant; false when no grant has that id.
    pub(crate) fn remove(&self, id: &str) -> bool {
        self.remove_many(std::slice::from_ref(&id.to_owned())) == 1
    }

    /// Removes every grant named in `ids` in one step ("Revoke all"); returns how many were removed. Ids that
    /// are already gone are skipped.
    pub(crate) fn remove_many(&self, ids: &[String]) -> usize {
        let mut state = self.lock();
        let before = state.kept.len();
        state.kept.retain(|k| !ids.contains(&k.grant.id));
        before - state.kept.len()
    }

    /// The session ended (deleted or archived): its grants, and those of its subagents' cards, end with it.
    /// True when a grant was removed.
    pub(crate) fn end_session(&self, session: &str) -> bool {
        let mut state = self.lock();
        state
            .pending
            .retain(|_, p| p.session != session && p.owner != session);
        let before = state.kept.len();
        state
            .kept
            .retain(|k| k.grant.session != session && k.owner != session);
        state.kept.len() != before
    }
}

/// The tool name a Chrome Control group grant keeps (`chrome:send`): never one of Vibe's tool names, so a Vibe
/// approval can never match it.
pub(crate) fn chrome_tool(key: &str) -> String {
    format!("chrome:{key}")
}

/// Whether a permission of approval `entry` is a literal scope whose invocation is not the pattern text itself
/// (see [`Grants::matching`]).
fn literal_differs_from_its_invocation(entry: &Value) -> bool {
    entry["detail"]["requiredPermissions"]
        .as_array()
        .is_some_and(|items| {
            items.iter().any(|item| {
                let (Some(scope), Some(invocation), Some(pattern)) = (
                    item["scope"].as_str(),
                    item["invocationPattern"].as_str(),
                    item["sessionPattern"].as_str(),
                ) else {
                    return false;
                };
                GrantPermission::from_wire(scope, invocation, pattern).exact
                    && invocation != pattern
            })
        })
}

/// The clients' view of the grants: every kept grant and the open cards' grant offers.
pub(crate) fn event(shared: &crate::core::Shared) -> crate::CoreEvent {
    crate::CoreEvent::AskGrants {
        grants: shared.ask_grants.list(),
        offers: shared.ask_grants.offers(),
    }
}

/// A plugin was removed or turned off: the grants for its tools end. Which routes a plugin published is
/// Vibe's own knowledge (a plugin's tool group, its MCP servers), so every grant for a tool that is neither one of
/// Vibe's builtins nor one of Vibe Work's own tools ends; a grant for another connector's tool is asked again too
/// (asking again is always safe). Tells the clients when one went.
pub(crate) fn plugin_changed(shared: &crate::core::Shared) {
    let builtin = |tool: &str| {
        tool.starts_with("file_system.")
            || tool.starts_with("process.")
            || tool == "skill.read"
            || tool.starts_with(OWN_TOOLS)
    };
    if shared.ask_grants.end_tools(|tool| !builtin(tool)) {
        shared.emit(event(shared));
    }
}

/// A connector was removed: the grants for its tools end with it, so a server added later under the same
/// name (maybe another address) is asked again. Tells the clients when one went.
pub(crate) fn connector_removed(shared: &crate::core::Shared) {
    if shared
        .ask_grants
        .end_tools(|tool| shared.connectors.removed_tool(tool))
    {
        shared.emit(event(shared));
    }
}

/// Ends the grants of a session that was deleted, archived or left the store, and tells the clients when one went.
pub(crate) fn session_ended(shared: &crate::core::Shared, session: &str) {
    if shared.ask_grants.end_session(session) {
        shared.emit(event(shared));
    }
}
