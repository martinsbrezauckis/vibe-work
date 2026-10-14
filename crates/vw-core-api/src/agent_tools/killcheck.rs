//! `kill_check`: the kill guard's question (classify, then fail closed):
//! classify, then fail closed). The hook classified a bash command; this decides on a fresh scan.
//!
//! Rules:
//! - a literal target of another session, another app, or Vibe Work -> **ask**, naming the owner;
//! - fully understood and every target this session's or not running -> **allow**;
//! - a recognized kill command with nonliteral targets -> **ask**: a native dialog shows the exact command; approved
//!   -> allow, else deny. **One ask per call (`one_ask`):** in every access mode (no list of profile
//!   names); a yes for a bash call is carried to Vibe's card for that call (the hook runs first; the card is then
//!   not shown); in our own `process_start`/`secret_run`, a yes the user gave on Vibe's card for this exact call
//!   (the card comes first) counts and the dialog is not shown again.
//!
//! A guard against mistakes, not a security boundary: without an OS sandbox an agent running as the user can
//! signal same-user processes by means no parser sees. Other sessions' processes are protected by
//! `process_stop` accepting only proven ownership, and by the user seeing what is held.
//!
//! Depends on: vw-procmon (scan, attribution), vw-agent-tools (`KillTarget`), vw-gate (confirmation), `session`
//! (names), `one_ask`. Called by: `server` (hook credential only), `tools` (`process_start`, `process_restart`:
//! the commands the core starts itself, which Vibe's bash hook never sees).
//! Invariant: nothing is signalled here.

use std::sync::Arc;

use serde::Deserialize;
use serde_json::{Value, json};
use vw_agent_tools::killparse::{KillScan, KillTarget, NameMatch, scan};
use vw_gate::{Confirmation, ConfirmationRequest, Confirmer};
use vw_procmon::{Attribution, Inputs, ProcessInfo, Snapshot};

use super::session::session_name;
use crate::core::Shared;
use crate::one_ask::Asked;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Question {
    #[serde(default)]
    session_id: String,
    #[serde(default)]
    parent_session_id: Option<String>,
    #[serde(default)]
    targets: Vec<KillTarget>,
    #[serde(default)]
    unparseable: Vec<String>,
    #[serde(default)]
    command: String,
    /// The hook's `tool_call_id` (empty from a hook that predates it: then Vibe's card is shown too).
    #[serde(default)]
    tool_call_id: String,
}

/// The method name a held command is shown under in the native dialog.
pub(crate) const HOLD_METHOD: &str = "vibe-work/killGuard";

/// Answers one `kill_check`: `{"decision": "allow"}` or `{"decision": "deny", "reason": ...}`. The command
/// is classified again here (the same classifier the hook ran), so the core decides on its own reading;
/// an old hook that sent no command is judged on the targets it sent.
pub(crate) async fn check(shared: &Arc<Shared>, args: Value) -> Result<Value, String> {
    let question: Question =
        serde_json::from_value(args).map_err(|_| "bad kill_check request".to_owned())?;
    let session = if shared.store.view(&question.session_id).is_some() {
        question.session_id.clone()
    } else {
        question
            .parent_session_id
            .clone()
            .filter(|p| shared.store.view(p).is_some())
            .unwrap_or_else(|| question.session_id.clone())
    };
    let found = if question.command.is_empty() {
        KillScan {
            mentions: true,
            targets: question.targets,
            unparseable: question.unparseable,
        }
    } else {
        scan(&question.command)
    };
    Ok(
        match guard(
            shared,
            &session,
            &question.command,
            &found,
            Asked::Hook {
                call_id: &question.tool_call_id,
                hook_session: &question.session_id,
            },
        )
        .await?
        {
            Ok(()) => json!({"decision": "allow"}),
            Err(reason) => json!({"decision": "deny", "reason": reason}),
        },
    )
}

/// The one kill-guard decision, shared by the bash hook (`kill_check`) and the commands the core starts
/// itself (`process_start`, `process_restart`), so the two can never drift. `Ok(Ok(()))` = run it,
/// `Ok(Err(reason))` = refused (the reason names the owner or the user's refusal); the outer error is an OS
/// scan failure. `asked`: who else asks the user about this call (module docs).
///
/// # Errors
/// A text reason when the process table cannot be read.
pub(crate) async fn guard(
    shared: &Arc<Shared>,
    session: &str,
    command: &str,
    found: &KillScan,
    asked: Asked<'_>,
) -> Result<Result<(), String>, String> {
    if !found.mentions {
        return Ok(Ok(()));
    }
    crate::processes::note_vibe_server(shared);
    let worker = shared.clone();
    let id = session.to_owned();
    let targets = found.targets.clone();
    let objections = tokio::task::spawn_blocking(move || objections(&worker, &id, &targets))
        .await
        .map_err(|e| format!("kill_check failed: {e}"))??;
    let mut reasons = found.unparseable.clone();
    reasons.extend(objections.into_iter().take(3));
    let (verdict, how) = if reasons.is_empty() {
        (Ok(()), "allowed")
    } else if matches!(asked, Asked::Card(true)) {
        // The user approved Vibe's card for this exact call of our own tool moments ago.
        (Ok(()), "allowed by the user on Vibe's card")
    } else if hold(shared, session, command, &reasons).await {
        if let Asked::Hook {
            call_id,
            hook_session,
        } = asked
        {
            // Vibe's card for this bash call (if its permissions ask) gets this yes from the core.
            if let Some(ledger) = crate::one_ask::of(shared) {
                ledger.guard_said_yes(&[hook_session, session], call_id, command);
            }
        }
        (Ok(()), "allowed by the user")
    } else {
        (
            Err("The user did not allow this process-stop command. \
                 Use processes_list and process_stop for this session's processes."
                .to_owned()),
            "refused by the user",
        )
    };
    tracing::info!(
        session,
        targets = found.targets.len(),
        unparseable = found.unparseable.len(),
        decision = how,
        "kill guard"
    );
    Ok(verdict)
}

/// Holds a command the classifier could not read for the user: a native dialog with the exact command.
async fn hold(shared: &Shared, session: &str, command: &str, why: &[String]) -> bool {
    let title = shared
        .names
        .title_of(&shared.store, session)
        .unwrap_or_else(|| session.to_owned());
    let request = ConfirmationRequest {
        method: HOLD_METHOD.to_owned(),
        requester: vw_gate::Requester::Agent,
        summary: hold_summary(&title, command, why),
        params: json!({
            "sessionId": session,
            "command": command,
            "why": why,
        }),
        always: None,
    };
    shared.confirmer.confirm(&request).await == Confirmation::Approved
}

fn hold_summary(title: &str, command: &str, why: &[String]) -> String {
    format!(
        "The agent of the session {} wants to run a command that may stop processes. It runs as you.\n\nWhy approval is needed: {}.\n\nCommand:\n{}",
        vw_gate::dialog_quote(title),
        why.iter()
            .map(|reason| vw_gate::dialog_line(reason))
            .collect::<Vec<_>>()
            .join("; "),
        vw_gate::dialog_quote(command),
    )
}

fn objections(
    shared: &Shared,
    session: &str,
    targets: &[KillTarget],
) -> Result<Vec<String>, String> {
    let snapshot = shared.procmon.scan().map_err(|e| e.to_string())?;
    let sessions = crate::processes::contexts(&shared.store);
    let inputs = shared.procmon.inputs(&sessions);
    let mut judge = Judge {
        shared,
        session,
        snapshot: &snapshot,
        inputs: &inputs,
        attribution: Attribution::new(&snapshot, &inputs),
        found: Vec::new(),
    };
    for target in targets {
        judge.target(target);
    }
    Ok(judge.found)
}

struct Judge<'a> {
    shared: &'a Shared,
    session: &'a str,
    snapshot: &'a Snapshot,
    inputs: &'a Inputs,
    attribution: Attribution<'a>,
    found: Vec<String>,
}

impl Judge<'_> {
    fn object(&mut self, text: String) {
        if !self.found.contains(&text) {
            self.found.push(text);
        }
    }

    fn target(&mut self, target: &KillTarget) {
        match target {
            KillTarget::Pid { pid } => self.pid(*pid),
            KillTarget::Group { pgid } => {
                if self.snapshot.me.pgid == Some(*pgid) {
                    self.object(format!("process group {pgid} is Vibe Work's own"));
                }
                let members: Vec<u32> = self
                    .snapshot
                    .processes
                    .values()
                    .filter(|p| p.alive && p.pgid == Some(*pgid))
                    .map(|p| p.pid)
                    .collect();
                for pid in members {
                    self.pid(pid);
                }
            }
            KillTarget::AllProcesses => self.object(
                "kill -1 signals every process of this user, other sessions and apps included"
                    .to_owned(),
            ),
            KillTarget::Name { name, how } => {
                let matching: Vec<u32> = self
                    .snapshot
                    .processes
                    .values()
                    .filter(|p| p.alive && p.same_user == Some(true) && matches(p, name, *how))
                    .map(|p| p.pid)
                    .collect();
                for pid in matching {
                    self.pid(pid);
                }
            }
            KillTarget::Port { port } => {
                let pids: Vec<Option<u32>> = self
                    .snapshot
                    .listeners
                    .iter()
                    .filter(|l| l.port == *port)
                    .map(|l| l.pid)
                    .collect();
                for pid in pids {
                    match pid {
                        Some(pid) => self.pid(pid),
                        None => {
                            self.object(format!("port {port} is held by another user's process"));
                        }
                    }
                }
            }
        }
    }

    fn pid(&mut self, pid: u32) {
        if pid <= 1 {
            self.object(format!("pid {pid} is not a process this session may stop"));
            return;
        }
        let Some(info) = self.snapshot.alive(pid) else {
            return; // not running: the kill fails on its own
        };
        let name = info.exe_name().unwrap_or_else(|| "unknown".to_owned());
        if self.snapshot.is_self_or_ancestor(pid) {
            self.object(format!("pid {pid} ({name}) is Vibe Work itself"));
            return;
        }
        if self.inputs.vibe_servers.iter().any(|s| s.pid == pid) {
            self.object(format!(
                "pid {pid} ({name}) is Vibe's app server, which runs every session"
            ));
            return;
        }
        let owner = self.attribution.process(pid).owner;
        match owner.session() {
            Some(s) if s == self.session => {}
            Some(other) => {
                let text = format!(
                    "pid {pid} ({name}) belongs to {}",
                    session_name(self.shared, other)
                );
                self.object(text);
            }
            None => self.object(format!("pid {pid} ({name}) belongs to another app")),
        }
    }
}

fn matches(process: &ProcessInfo, pattern: &str, how: NameMatch) -> bool {
    let pattern_lower = pattern.to_ascii_lowercase();
    let stem = pattern_lower
        .strip_suffix(".exe")
        .unwrap_or(&pattern_lower)
        .to_owned();
    let exe = process
        .exe_name()
        .map(|n| n.to_ascii_lowercase())
        .unwrap_or_default();
    let exe_stem = exe.strip_suffix(".exe").unwrap_or(&exe).to_owned();
    let comm = process.name.to_ascii_lowercase();
    match how {
        NameMatch::Exact => exe_stem == stem || comm == stem,
        NameMatch::Substring => exe.contains(&pattern_lower) || comm.contains(&pattern_lower),
    }
}

#[cfg(test)]
mod kg1_tests {
    use super::hold_summary;

    #[test]
    fn held_kill_command_is_complete() {
        let command = format!("kill {}", "123 ".repeat(600));
        let text = hold_summary("Build session", &command, &["other session PID".to_owned()]);
        assert!(text.contains("Build session"));
        assert!(text.contains("other session PID"));
        assert!(text.ends_with(&format!("Command:\n{}", vw_gate::dialog_quote(&command))));
    }
}
