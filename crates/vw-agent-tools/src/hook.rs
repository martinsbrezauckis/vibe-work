//! The `pre_tool` guards in Vibe Work's `$VIBE_HOME/hooks.toml` (Vibe runs them before every matching tool call,
//! also in auto-approve): the safety reminders ([`crate::safety`]), the process (kill) guard, the git
//! guard, and the folder guard ([`crate::folders`]) and the connector guard
//! ([`crate::forge`]).
//!
//! each guard is its own `[[hooks]]` entry with its own name ([`Guard::hook_name`]), because
//! Vibe labels a refusal with the hook's name ("vibe-work git guard: Denied tool 'bash'"); one shared name made a
//! push refusal read as a kill-guard refusal. [`decide`] still runs all of them in order (tests, the old
//! `kill-guard` command line).
//!
//! Git guard ([`crate::gitparse`]): literal Git and GitHub CLI command starts are classified.
//! Pushes, forge writes and credential commands are refused here; branch rewrites ask the core
//! (`git_check`) to compare with the session's copy branch.
//!
//! Kill guard: It reads Vibe's hook payload on stdin, classifies the
//! command ([`crate::killparse`]: untouched / understood with literal targets / not understood) and asks the
//! core: literal targets are allowed when all are this session's; foreign or nonliteral targets are held for
//! the user (a native dialog in the core). A guard against mistakes, not
//! a security boundary (see `killparse`).
//!
//! One ask per call: Vibe runs this hook BEFORE its own approval card,
//! so the hook passes the call's `tool_call_id` to the core; a yes in the core's dialog then answers Vibe's card for
//! the same call (the core's `one_ask`), and the user is asked once.
//!
//! Folder guard: paths inside a session copy (any tool's path fields, bash paths) are allowed only when the core says
//! this session may work in that copy now (`copy_check`: one active session per copy; no answer
//! about any copy = refused). Everything else is decided here without the
//! core: the Vibe home comes from `VIBE_HOME` in the environment Vibe gives the hook, the own copy from the payload's
//! `cwd`.
//!
//! Connector guard: a removed MCP server's stale tools are refused even while an open turn cannot refresh. A
//! forge-writing connector call (`connector_github_app.push_files`) is refused here only when Vibe's provider is
//! not our model proxy; with the proxy (every Vibe Work build that holds the key) the call is let through to the
//! proxy, which answers it with the Push-button sentence as an ordinary tool result, so the model reads it
//! wherever the call was made.
//!
//! Hook protocol (Vibe 2.26.0 `_foreign_hooks.py`): stdin = `{session_id, parent_session_id, cwd, tool_name,
//! tool_call_id, tool_input: {command, ...}, ...}`; stdout empty = allow; `{"decision": "deny", "reason": "..."}` = the call
//! is skipped and the reason reaches the model. The hook is declared `strict`, so a crash also denies.
//!
//! Depends on: `killparse`, `gitparse`, `folders`, `forge`, `safety`, `wire`, `serde_json`. Called by: `cli.rs`
//! (`vw-agent-tools guard <name>` / `kill-guard`), tests.
//! Invariants:
//! - A command with no literal process-stop command start is allowed without contacting the core (the
//!   common case costs one process start and a scan).
//! - Anything that goes wrong after a kill command was found denies (never allow blindly).

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::killparse::scan;
use crate::safety;
use crate::wire::{self, tools};
use crate::{folders, forge, gitparse};

/// The file the core writes at start for the hook: its listener and the hook credential (user-only).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndpointFile {
    /// `127.0.0.1:<port>`.
    pub endpoint: String,
    /// The hook credential.
    pub credential: String,
}

/// Person-waits have no transport deadline. Stop closes their connection by killing the hook.
pub const HOLD_TIMEOUT: Option<std::time::Duration> = None;

/// The folder guard's answers of one hook run: copy -> the core's answer.
type Answers = Vec<(std::path::PathBuf, Result<(), String>)>;

/// How long the folder guard waits for the core's answer about another session's copy (a lookup, no dialog).
pub const COPY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Longest hook payload read from stdin.
pub const MAX_PAYLOAD: usize = 1024 * 1024;

const ADVICE: &str = "Use the Vibe Work process tools instead: processes_list shows this session's processes, \
process_stop stops one of them.";

/// The result of a call the user said No to: the same text as `vw_ask::DECLINED`, which the core returns.
pub const DECLINED: &str = "Not run: the user declined this. Do not do it another way. Tell the user why it is needed and wait for their reply before you go on with it.";

/// The kill guard's refusal. A guard refusal names the process tools; the user's own No is returned as it is,
/// so nothing points the agent at another way to stop the same process.
fn kill_refusal(reason: &str) -> String {
    if reason == DECLINED {
        return deny(reason);
    }
    deny(&format!("{reason} {ADVICE}"))
}

fn deny(reason: &str) -> String {
    json!({"decision": "deny", "reason": reason}).to_string()
}

/// The refusal of a push: positive words, the outcome first (models read "refused" as their own failure).
pub const PUSH_REASON: &str = "Not run: sending commits to a remote is the user's step in Vibe Work. Your commits \
                               stay on this branch. Tell the user the work is committed, name its folder, and that \
                               selecting that folder and pressing Push in the Changes tab sends it.";

/// One guard of the hook block (one `[[hooks]]` entry each, so Vibe's notice names the right one).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Guard {
    /// Safety reminders: deleting the home folder, disk tools, shutdown, the user's own rules.
    Safety,
    /// The kill guard: other sessions' and apps' processes.
    Process,
    /// The git guard: remote writes are the user's buttons; other-branch rewrites ask.
    Git,
    /// The folder guard: other sessions' copies.
    Folder,
    /// The connector guard: forge-writing connector tools.
    Connector,
    /// Binds an MCP call to the Vibe source and call ID, using the private hook credential.
    Receipt,
}

impl Guard {
    /// Every guard, in the order the hook block lists them (the first refusal wins in Vibe).
    pub const ALL: [Self; 6] = [
        Self::Safety,
        Self::Process,
        Self::Folder,
        Self::Git,
        Self::Connector,
        Self::Receipt,
    ];

    /// The name on the command line (`vw-agent-tools guard <arg>`).
    #[must_use]
    pub const fn arg(self) -> &'static str {
        match self {
            Self::Safety => "safety",
            Self::Process => "process",
            Self::Git => "git",
            Self::Folder => "folder",
            Self::Connector => "connector",
            Self::Receipt => "receipt",
        }
    }

    /// The guard for a command-line name.
    #[must_use]
    pub fn from_arg(arg: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|g| g.arg() == arg)
    }

    /// The hook's name in `hooks.toml`: what Vibe shows next to a refusal ("vibe-work git guard: Denied tool 'bash'").
    #[must_use]
    pub const fn hook_name(self) -> &'static str {
        match self {
            Self::Safety => "vibe-work safety reminder",
            Self::Process => "vibe-work process guard",
            Self::Git => "vibe-work git guard",
            Self::Folder => "vibe-work folder guard",
            Self::Connector => "vibe-work connector guard",
            Self::Receipt => "vibe-work tool receipt",
        }
    }

    /// The tools the hook entry matches (Vibe's `match`: a glob, or `re:` = a regex FULL-matched against the tool's
    /// qualified name only; Vibe 2.26.1 `utils/matching.py::name_matches`).
    ///
    /// The command guards match every tool (fail closed, like the folder guard): Vibe's hooked shells are not only
    /// `bash` and `process_start` but also `powershell` / `powershell_stdin` (Vibe's managed Windows shell, switched on
    /// by `managed_shell_tools_enabled` or a Vibe experiment) and any later name; the guards act only on a call that
    /// carries `tool_input.command`, so other tools pass unchanged (one process start each).
    ///
    /// The hooks are one layer: the agent's environment (no credentials, pushes unreachable, gh signed out)
    /// and Vibe's own approval cards apply to every command as well.
    #[must_use]
    pub const fn matcher(self) -> &'static str {
        match self {
            // Every tool. Command guards: see above. Folder guard: any tool input that names a path is checked
            // Unknown tools fail closed.
            Self::Safety | Self::Process | Self::Git | Self::Folder => "*",
            Self::Connector => "re:connector_.+|mcp_.+",
            Self::Receipt => "mcp_vw.*",
        }
    }

    /// Person-waits have no timeout; Vibe cancels their process when the turn stops.
    #[must_use]
    pub const fn timeout_secs(self) -> f64 {
        match self {
            Self::Process | Self::Git | Self::Receipt => f64::INFINITY,
            _ => 25.0,
        }
    }

    /// Decides one hook call for this guard. `None` = allow (print nothing).
    #[must_use]
    pub fn decide(self, payload: &str, endpoint_file: &Path) -> Option<String> {
        let answer = match self {
            Self::Safety => decide_safety(payload, endpoint_file),
            Self::Process => decide_kill(payload, endpoint_file),
            Self::Git => decide_git(payload, endpoint_file),
            Self::Folder => decide_folder(payload, endpoint_file),
            Self::Connector => decide_connector(payload, endpoint_file),
            // The core records its own outcomes (a Never in the bell, the user's No only in the session).
            Self::Receipt => return crate::receipt_hook::decide(payload, endpoint_file),
        };
        answer.map(|answer| report_refusal(self, payload, endpoint_file, answer))
    }
}

/// Decides one hook call with every guard in order. Returns the stdout to print: `None` = allow (print nothing).
#[must_use]
pub fn decide(payload: &str, endpoint_file: &Path) -> Option<String> {
    let mut payload = payload.to_owned();
    let mut rewritten = None;
    for guard in Guard::ALL {
        if let Some(answer) = guard.decide(&payload, endpoint_file) {
            // Fail closed: an answer that is not a rewrite (or cannot be read as one) is printed as it is.
            let Ok(output) = serde_json::from_str::<Value>(&answer) else {
                return Some(answer);
            };
            let Some(input) = output.pointer("/hook_specific_output/tool_input") else {
                return Some(answer);
            };
            let Ok(mut next) = serde_json::from_str::<Value>(&payload) else {
                return Some(answer);
            };
            next["tool_input"] = input.clone();
            payload = next.to_string();
            rewritten = Some(answer);
        }
    }
    rewritten
}

/// The folder guard's half: Vibe's file tools and bash paths inside another session's copy.
fn decide_folder(payload: &str, endpoint_file: &Path) -> Option<String> {
    let payload = serde_json::from_str::<Value>(payload).ok()?;
    let tool = payload.get("tool_name").and_then(Value::as_str)?;
    // Our own `mcp_vw.*` tools run only inside the session's folders (`session::inside` in the core).
    if tool.starts_with(&format!("mcp_{}.", crate::wire::SERVER_NAME)) {
        return None;
    }
    // Windows: a file tool's Git Bash paths (`/c/Users/...`) are rewritten as Windows paths (`C:\Users\...`).
    // Return the same input the folder guard checked, using the receipt hook's rewrite protocol.
    #[cfg(windows)]
    let rewritten = crate::msys_paths::rewrite(tool, &payload["tool_input"]);
    #[cfg(not(windows))]
    let rewritten: Option<Value> = None;
    if rewritten.is_some()
        && (payload["cwd"].as_str().is_none_or(str::is_empty)
            || std::env::var_os("VIBE_HOME").is_none())
    {
        return Some(deny(
            "Not run: the folder guard could not check the native Windows path.",
        ));
    }
    let cwd = payload
        .get("cwd")
        .and_then(Value::as_str)
        .filter(|c| !c.is_empty())?;
    let vibe_home = std::env::var_os("VIBE_HOME").map(std::path::PathBuf::from)?;
    let layout = match folders::Layout::new(&vibe_home, Path::new(cwd)) {
        Ok(layout) => layout,
        Err(error) => {
            return Some(deny(&format!(
                "Not run: the folder guard could not resolve the session folders: {error}"
            )));
        }
    };
    let leaf = tool.rsplit('.').next().unwrap_or(tool);
    let input = rewritten.as_ref().or_else(|| payload.get("tool_input"))?;
    let access = match leaf {
        "read_file" => folders::Access::Read,
        "write_file" | "edit" | "search_replace" => folders::Access::Write,
        _ => folders::Access::Unknown,
    };
    // Any copy (the own one too): the core decides (one active session per copy). One question per copy and hook run.
    // No answer: every copy is refused because a different active session may have taken even the own copy.
    let asked: std::cell::RefCell<Answers> = std::cell::RefCell::default();
    let permits = |copy: &Path, own: bool| -> Result<(), String> {
        if let Some((_, answer)) = asked.borrow().iter().find(|(c, _)| c == copy) {
            return answer.clone();
        }
        let args = json!({
            "sessionId": payload.get("session_id").and_then(Value::as_str).unwrap_or(""),
            "parentSessionId": payload.get("parent_session_id").and_then(Value::as_str),
            "copy": copy.to_string_lossy(),
            "own": own,
        });
        let answer = match read_endpoint(endpoint_file).and_then(|file| {
            wire::call(
                &file.endpoint,
                &file.credential,
                tools::COPY_CHECK,
                args,
                COPY_TIMEOUT,
            )
        }) {
            Ok(value) if value.get("decision").and_then(Value::as_str) == Some("allow") => Ok(()),
            Ok(value) => Err(value
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("Vibe Work did not allow it")
                .to_owned()),
            Err(problem) => Err(format!(
                "Vibe Work could not check who uses this copy ({problem})"
            )),
        };
        asked
            .borrow_mut()
            .push((copy.to_path_buf(), answer.clone()));
        answer
    };
    let mut refusal = folders::check_tool_input(&layout, input, access, &permits);
    if refusal.is_none() && folders::SHELL_TOOLS.contains(&leaf) {
        let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
        refusal = input
            .get("command")
            .and_then(Value::as_str)
            .and_then(|command| {
                folders::check_command(&layout, command, home.as_deref(), &permits)
            });
    }
    refusal.map(|text| deny(&text)).or_else(|| {
        rewritten.map(|input| json!({"hook_specific_output":{"tool_input":input}}).to_string())
    })
}

/// True when Vibe's Mistral provider is our loopback model proxy (`VIBE_PROVIDERS` set by vw-keys `vibe_env`):
/// then every connector call passes the proxy, which applies the connector rule itself.
fn proxy_in_charge() -> bool {
    let Some(text) = std::env::var_os("VIBE_PROVIDERS") else {
        return false;
    };
    let Ok(Value::Array(providers)) = serde_json::from_str::<Value>(&text.to_string_lossy()) else {
        return false;
    };
    providers.iter().any(|p| {
        p.get("name").and_then(Value::as_str) == Some("mistral")
            && p.get("api_base")
                .and_then(Value::as_str)
                .is_some_and(|base| base.starts_with("http://127.0.0.1:"))
    })
}

/// The connector guard's half: a forge-writing connector tool is refused here when the model proxy is
/// not in charge (it answers the call itself otherwise, in words the model can read).
fn decide_connector(payload: &str, endpoint_file: &Path) -> Option<String> {
    let payload = serde_json::from_str::<Value>(payload).ok()?;
    let tool = payload.get("tool_name").and_then(Value::as_str)?;
    if tool.starts_with("mcp_") && !tool.starts_with("mcp_vw.") {
        let result = read_endpoint(endpoint_file).and_then(|file| {
            wire::call(
                &file.endpoint,
                &file.credential,
                tools::CONNECTOR_CHECK,
                json!({"tool": tool}),
                COPY_TIMEOUT,
            )
        });
        return match result {
            Ok(value) if value.get("removed").and_then(Value::as_bool) == Some(true) => Some(deny(
                "Not run: this MCP server was removed in Vibe Work. Its tools may remain listed in this open chat until it refreshes.",
            )),
            Ok(value) if value.get("removed").and_then(Value::as_bool) == Some(false) => None,
            _ => Some(deny(
                "Not run: Vibe Work could not check whether this MCP server is still available.",
            )),
        };
    }
    match forge::refuses_tool_name(tool) {
        Some(true) if !proxy_in_charge() => Some(deny(forge::CONNECTOR_REASON)),
        _ => None,
    }
}

/// The safety reminders and exact command observation before Vibe's display projection.
/// Our own `mcp_vw.*` tools are checked by the core itself.
fn decide_safety(payload: &str, endpoint_file: &Path) -> Option<String> {
    let payload = serde_json::from_str::<Value>(payload).ok()?;
    let own_tool = format!("mcp_{}.", crate::wire::SERVER_NAME);
    if payload
        .get("tool_name")
        .and_then(Value::as_str)
        .is_some_and(|t| t.starts_with(&own_tool))
    {
        return None;
    }
    let command = payload
        .pointer("/tool_input/command")
        .and_then(Value::as_str)?;
    // A failed observation grants nothing: a projected search card without its command is refused.
    let _ = read_endpoint(endpoint_file).and_then(|file| wire::call(
        &file.endpoint, &file.credential, tools::SHELL_COMMAND,
        json!({"sessionId":payload["session_id"], "toolCallId":payload["tool_call_id"], "command":command}),
        std::time::Duration::from_secs(2),
    ));
    let cwd = payload
        .get("cwd")
        .and_then(Value::as_str)
        .filter(|c| !c.is_empty())
        .map(Path::new);
    let settings = safety::load(&safety::settings_file(endpoint_file));
    safety::check(command, cwd, &settings).map(|stop| deny(&stop.text))
}

/// The git guard's half of [`decide`].
fn decide_git(payload: &str, endpoint_file: &Path) -> Option<String> {
    let payload = serde_json::from_str::<Value>(payload).ok()?;
    let own_tool = format!("mcp_{}.", crate::wire::SERVER_NAME);
    if payload
        .get("tool_name")
        .and_then(Value::as_str)
        .is_some_and(|t| t.starts_with(&own_tool))
    {
        // `mcp_vw.process_start`: the core runs the same git check itself before it starts anything.
        return None;
    }
    let command = payload
        .pointer("/tool_input/command")
        .and_then(Value::as_str)?;
    let found = gitparse::scan(command);
    if !found.needs_core() {
        return None;
    }
    if found
        .ops
        .iter()
        .any(|op| op.kind == gitparse::GitKind::Deny && !op.what.starts_with("gh "))
    {
        return Some(deny(PUSH_REASON));
    }
    if found
        .ops
        .iter()
        .any(|op| op.kind == gitparse::GitKind::Credential && !op.what.starts_with("gh "))
    {
        return Some(deny(gitparse::CREDENTIAL_REASON));
    }
    let args = json!({
        "sessionId": payload.get("session_id").and_then(Value::as_str).unwrap_or(""),
        "parentSessionId": payload.get("parent_session_id").and_then(Value::as_str),
        "cwd": payload.get("cwd").and_then(Value::as_str).unwrap_or(""),
        "command": command,
        // the call's id, so a yes in the core's dialog answers Vibe's card for this same call.
        "toolCallId": payload.get("tool_call_id").and_then(Value::as_str).unwrap_or(""),
    });
    let answer = read_endpoint(endpoint_file).and_then(|file| {
        wire::call(
            &file.endpoint,
            &file.credential,
            tools::GIT_CHECK,
            args,
            HOLD_TIMEOUT,
        )
    });
    match answer {
        Ok(value) if value.get("decision").and_then(Value::as_str) == Some("allow") => None,
        Ok(value) => {
            Some(deny(value.get("reason").and_then(Value::as_str).unwrap_or(
                "Not run: Vibe Work did not allow this git command.",
            )))
        }
        Err(problem) => Some(deny(&format!(
            "Not run: Vibe Work could not check this git command ({problem}). Ask the user to run it."
        ))),
    }
}

pub(crate) fn read_endpoint(endpoint_file: &Path) -> Result<EndpointFile, String> {
    std::fs::read_to_string(endpoint_file)
        .map_err(|e| format!("endpoint file unreadable: {e}"))
        .and_then(|text| {
            serde_json::from_str::<EndpointFile>(&text)
                .map_err(|_| "endpoint file is not valid".to_owned())
        })
}

/// The kill guard's half of [`decide`].
fn decide_kill(payload: &str, endpoint_file: &Path) -> Option<String> {
    let Ok(payload) = serde_json::from_str::<Value>(payload) else {
        // Not Vibe's payload: nothing to check. Vibe always sends JSON; the strict flag covers crashes.
        return None;
    };
    // Our own `mcp_vw.process_start`: the core runs the same decision itself before it starts anything
    // (`kill_check`'s shared `guard`), so the hook stays out (one dialog, not two).
    let own_tool = format!("mcp_{}.", crate::wire::SERVER_NAME);
    if payload
        .get("tool_name")
        .and_then(Value::as_str)
        .is_some_and(|t| t.starts_with(&own_tool))
    {
        return None;
    }
    let command = payload
        .pointer("/tool_input/command")
        .and_then(Value::as_str)?;
    let found = scan(command);
    // Understood with no target (`grep -r kill src`, `kill -l`): nothing to check.
    if !found.mentions || (found.understood() && found.targets.is_empty()) {
        return None;
    }
    let session = payload
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    let parent = payload.get("parent_session_id").and_then(Value::as_str);
    let args = json!({
        "sessionId": session,
        "parentSessionId": parent,
        "targets": found.targets,
        "unparseable": found.unparseable,
        // The exact text, for the user's dialog when the core holds the command.
        "command": command,
        // the call's id, so a yes in the core's dialog answers Vibe's card for this same call.
        "toolCallId": payload.get("tool_call_id").and_then(Value::as_str).unwrap_or(""),
    });
    let answer = std::fs::read_to_string(endpoint_file)
        .map_err(|e| format!("endpoint file unreadable: {e}"))
        .and_then(|text| {
            serde_json::from_str::<EndpointFile>(&text)
                .map_err(|_| "endpoint file is not valid".to_owned())
        })
        .and_then(|file| {
            wire::call(
                &file.endpoint,
                &file.credential,
                tools::KILL_CHECK,
                args,
                HOLD_TIMEOUT,
            )
        });
    match answer {
        Ok(value) if value.get("decision").and_then(Value::as_str) == Some("allow") => None,
        Ok(value) => Some(kill_refusal(
            value
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("Vibe Work blocked this kill command."),
        )),
        Err(problem) => Some(deny(&format!(
            "Vibe Work could not check which processes this kill command targets ({problem}). {ADVICE}"
        ))),
    }
}

// Report only the final decision: core-assisted checks must not publish a second event.
fn report_refusal(guard: Guard, payload: &str, endpoint_file: &Path, answer: String) -> String {
    let Ok(payload) = serde_json::from_str::<Value>(payload) else {
        return answer;
    };
    let Ok(mut decision) = serde_json::from_str::<Value>(&answer) else {
        return answer;
    };
    if decision["decision"] != "deny" {
        return answer;
    }
    let kind = match guard {
        Guard::Git => "git",
        Guard::Connector
            if payload
                .get("tool_name")
                .and_then(Value::as_str)
                .and_then(forge::refuses_tool_name)
                == Some(true) =>
        {
            "git"
        }
        Guard::Connector | Guard::Safety => "safetyRule",
        Guard::Folder => "folder",
        Guard::Process => "process",
        Guard::Receipt => return answer,
    };
    let result = read_endpoint(endpoint_file).and_then(|file| wire::call(
        &file.endpoint, &file.credential, tools::GUARD_REFUSED,
        json!({ "sessionId": payload["session_id"].as_str().unwrap_or(""), "parentSessionId": payload["parent_session_id"],
          "guard": kind, "toolCallId": payload["tool_call_id"], "reason": decision["reason"] }), COPY_TIMEOUT));
    match result {
        Ok(value) => {
            if let Some(reason) = value.get("reason").and_then(Value::as_str) {
                decision["reason"] = Value::String(reason.to_owned());
            }
        }
        Err(error) => eprintln!("guard refusal could not be published: {error}"),
    }
    decision.to_string()
}

#[cfg(test)]
mod kill_refusal_tests {
    use super::*;

    #[test]
    fn a_user_no_is_returned_as_it_is_and_a_guard_refusal_gets_the_process_tools() {
        assert_eq!(kill_refusal(DECLINED), deny(DECLINED));
        let refused = "Not run: pid 7 belongs to another app.";
        assert_eq!(kill_refusal(refused), deny(&format!("{refused} {ADVICE}")));
    }
}
