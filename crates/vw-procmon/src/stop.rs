//! Safe stop : stop a process and its group (Unix) or tree (Windows) only
//! when a FRESH scan proves it is what the caller may stop, then TERM -> wait -> KILL.
//!
//! Depends on: `os` (scan, liveness), `attribution`, `types`, nix (Unix signals), vw-platform (Windows
//! handle-pinned terminate). Called by: `monitor` (explicit stops, session end, quit, startup sweep).
//! Refusal order (every refusal leaves the process untouched): pid outside 2..=`i32::MAX` (0 = own group,
//! 1 = init, larger values turn negative in `kill(2)`; ShellX `checked_unix_process_id` @ e38c2b5c) ->
//! gone (not an error) -> no start token -> start token differs (pid reused) -> another OS user -> this app
//! or an ancestor -> authority ([`Authority`]: a session needs a FACT owner, the user any session owner) ->
//! Unix: the group is the app's own group, or a live group member that is not the target's descendant fails
//! the same authority check.
//! Linux closes the pid-reuse window between the check and the signal with a pidfd (`pidfd_open`, start
//! token re-checked after opening, `pidfd_send_signal`) for the target; a group is signalled with `killpg`,
//! which cannot hit a reused group id while a verified member is alive (a group id stays allocated while any
//! member exists), and is re-verified before KILL.
//! Windows has no TERM for console processes: the tree is terminated at once, each process through a handle
//! on which its start token was checked (proven on Windows 11 only).

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::attribution::{Attributed, Attribution, Confidence, Inputs, Owner};
use crate::error::ScanError;
use crate::os;
use crate::signal;
use crate::types::{ProcessRef, Snapshot};

/// Stop timing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StopConfig {
    /// Wait after TERM before KILL. Default 3 s.
    pub term_grace: Duration,
    /// Wait after KILL. Default 2 s.
    pub kill_grace: Duration,
    /// Poll interval while waiting. Default 25 ms.
    pub poll: Duration,
}

impl Default for StopConfig {
    fn default() -> Self {
        Self {
            term_grace: Duration::from_secs(3),
            kill_grace: Duration::from_secs(2),
            poll: Duration::from_millis(25),
        }
    }
}

/// On whose authority a stop runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Authority {
    /// A session itself (its agent through the tool, or an automatic session action): the target must
    /// be attributed to that session **as a fact** (registered by this app, re-attached from the ledger, or a
    /// descendant of such a process). An owner that is only inferred (cwd in the folder, group membership)
    /// is refused: it may be the user's own process.
    Session(String),
    /// The user, after the UI showed name, pid and owner and the user confirmed: any process attributed to
    /// one of our sessions (fact or inferred). Never another app, another user's process or this app
    /// ("stop the owner only if it is one of ours").
    User,
    /// The process ledger: the identity was recorded by this app while it ran (startup sweep, quit).
    Ledger,
}

/// Why a stop was refused.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum StopRefusal {
    /// The pid can never be a safe target.
    #[error("pid {pid} is not a valid stop target")]
    InvalidPid {
        /// The pid.
        pid: u64,
    },
    /// The OS gives no start time, so pid reuse cannot be ruled out.
    #[error("pid {pid} has no readable start time")]
    NoStartToken {
        /// The pid.
        pid: u32,
    },
    /// Another process now has this pid.
    #[error("pid {pid} now belongs to another process (pid reused)")]
    PidReused {
        /// The pid.
        pid: u32,
    },
    /// Another OS user's process.
    #[error("pid {pid} belongs to another user")]
    ForeignUser {
        /// The pid.
        pid: u32,
    },
    /// This app or one of its ancestors.
    #[error("pid {pid} is this app or its parent")]
    ThisApp {
        /// The pid.
        pid: u32,
    },
    /// Attributed to someone else.
    #[error("pid {pid} does not belong to this session")]
    #[serde(rename_all = "camelCase")]
    NotThisSession {
        /// The pid.
        pid: u32,
        /// Its owner.
        owner: Owner,
    },
    /// Attributed to the session only by inference (cwd, parent), not proven started by this app or the
    /// agent: a session may not stop it; the user may, after confirming.
    #[error("pid {pid} is not proven to be this session's (started outside Vibe Work?)")]
    #[serde(rename_all = "camelCase")]
    NotProven {
        /// The pid.
        pid: u32,
        /// The (inferred) owner.
        owner: Owner,
    },
    /// Its process group is the app's own group.
    #[error("pid {pid} is in this app's process group {pgid}")]
    SharesAppGroup {
        /// The pid.
        pid: u32,
        /// The group.
        pgid: u32,
    },
    /// Not a process a deleted session left running: the "Left running" list stops only identities it
    /// holds.
    #[error("pid {pid} is not a process a deleted session left running")]
    NotLeftRunning {
        /// The pid.
        pid: u32,
    },
    /// A member of its group is not ours to stop.
    #[error("process group of pid {pid} also holds pid {member}, which is not this session's")]
    #[serde(rename_all = "camelCase")]
    GroupShared {
        /// The pid.
        pid: u32,
        /// The foreign member.
        member: u32,
        /// The member's owner.
        owner: Owner,
    },
}

/// Why a stop failed.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum StopError {
    /// Refused before anything was signalled.
    #[error(transparent)]
    Refused(#[from] StopRefusal),
    /// The OS tables could not be read.
    #[error(transparent)]
    Scan(#[from] ScanError),
    /// Signalling failed.
    #[error("signal to {pid} failed: {reason}")]
    Signal {
        /// The pid or group.
        pid: u32,
        /// The OS error.
        reason: String,
    },
}

/// What a stop did.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StopOutcome {
    /// The target.
    pub target: ProcessRef,
    /// It was already gone; nothing was signalled.
    pub already_gone: bool,
    /// The Unix process group signalled.
    pub group: Option<u32>,
    /// Pids that were stopped together (group members or tree).
    pub members: Vec<u32>,
    /// KILL was needed after TERM.
    pub escalated: bool,
    /// Every member is gone.
    pub gone: bool,
    /// Members still alive at the end.
    pub remaining: Vec<u32>,
    /// Ports the members listened on before the stop.
    pub ports: Vec<u16>,
    /// Milliseconds the stop took.
    pub took_ms: u64,
}

/// Stops `target` (and its group / tree) on `authority`. Blocking (sleeps while waiting); run it off the
/// async runtime.
///
/// # Errors
/// [`StopError::Refused`] with the reason before anything is signalled; `Scan` / `Signal` on OS failures.
pub fn stop(
    target: ProcessRef,
    authority: &Authority,
    inputs: &Inputs,
    config: &StopConfig,
) -> Result<StopOutcome, StopError> {
    let started = Instant::now();
    if !(2..=i32::MAX.unsigned_abs()).contains(&target.pid) {
        return Err(StopRefusal::InvalidPid {
            pid: u64::from(target.pid),
        }
        .into());
    }
    let snapshot = os::snapshot()?;
    let Some(info) = snapshot.alive(target.pid) else {
        return Ok(gone_outcome(target, started));
    };
    let pid = target.pid;
    match info.start {
        None => return Err(StopRefusal::NoStartToken { pid }.into()),
        Some(start) if start != target.start => return Err(StopRefusal::PidReused { pid }.into()),
        Some(_) => {}
    }
    if info.same_user != Some(true) {
        return Err(StopRefusal::ForeignUser { pid }.into());
    }
    if snapshot.is_self_or_ancestor(pid) {
        return Err(StopRefusal::ThisApp { pid }.into());
    }
    let mut attribution = Attribution::new(&snapshot, inputs);
    authorize(pid, &attribution.process(pid), authority)?;
    let plan = plan_members(&snapshot, target, info.pgid, authority, &mut attribution)?;
    let ports: Vec<u16> = plan
        .members
        .iter()
        .flat_map(|m| snapshot.ports_of(m.pid))
        .collect();
    let (escalated, remaining) = signal::run(&plan, config)?;
    Ok(StopOutcome {
        target,
        already_gone: false,
        group: plan.group,
        members: plan.members.iter().map(|m| m.pid).collect(),
        escalated,
        gone: remaining.is_empty(),
        remaining,
        ports,
        took_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
    })
}

/// The authority check for one process (the target, or a group member that is not its descendant).
pub(crate) fn authorize(
    pid: u32,
    result: &Attributed,
    authority: &Authority,
) -> Result<(), StopRefusal> {
    let owner = result.owner.clone();
    match authority {
        Authority::Session(session) => {
            if owner.session() != Some(session.as_str()) {
                return Err(StopRefusal::NotThisSession { pid, owner });
            }
            if result.confidence != Some(Confidence::Fact) {
                return Err(StopRefusal::NotProven { pid, owner });
            }
            Ok(())
        }
        Authority::User => {
            if owner.session().is_none() {
                return Err(StopRefusal::NotThisSession { pid, owner });
            }
            Ok(())
        }
        Authority::Ledger => Ok(()),
    }
}

fn gone_outcome(target: ProcessRef, started: Instant) -> StopOutcome {
    StopOutcome {
        target,
        already_gone: true,
        group: None,
        members: Vec::new(),
        escalated: false,
        gone: true,
        remaining: Vec::new(),
        ports: Vec::new(),
        took_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
    }
}

/// What will be signalled.
pub(crate) struct Plan {
    /// Unix group to signal (Windows: `None`, the members are terminated one by one).
    pub(crate) group: Option<u32>,
    /// Every process that must be gone afterwards (target first).
    pub(crate) members: Vec<ProcessRef>,
    /// The target.
    pub(crate) target: ProcessRef,
    /// Process names of the members (for error texts; Windows names the member a terminate failed on).
    #[cfg_attr(
        not(windows),
        expect(dead_code, reason = "only the Windows signal path reports names")
    )]
    pub(crate) names: std::collections::HashMap<u32, String>,
}

/// Unix: the target's group (refusing the app's own group and groups with foreign members). Windows: the
/// target and its start-consistent descendants.
fn plan_members(
    snapshot: &Snapshot,
    target: ProcessRef,
    pgid: Option<u32>,
    authority: &Authority,
    attribution: &mut Attribution<'_>,
) -> Result<Plan, StopRefusal> {
    let target_pid = target.pid;
    let unix_group = if cfg!(unix) { pgid } else { None };
    if let Some(g) = unix_group {
        if Some(g) == snapshot.me.pgid {
            return Err(StopRefusal::SharesAppGroup {
                pid: target_pid,
                pgid: g,
            });
        }
        if g <= 1 {
            return Err(StopRefusal::InvalidPid { pid: u64::from(g) });
        }
    }
    let mut members = vec![target];
    for process in snapshot
        .processes
        .values()
        .filter(|p| p.alive && p.pid != target_pid)
    {
        let in_scope = match unix_group {
            Some(g) => process.pgid == Some(g),
            None => snapshot.descends_from(process.pid, target),
        };
        if !in_scope {
            continue;
        }
        let member = process.pid;
        if process.same_user != Some(true) || snapshot.is_self_or_ancestor(member) {
            return Err(StopRefusal::GroupShared {
                pid: target_pid,
                member,
                owner: attribution.process(member).owner,
            });
        }
        // A descendant of the target belongs to the tree being stopped; any other group member must pass
        // the same authority check as the target (facts for a session, any session for the user); the
        // ledger authority knows no sessions and refuses it.
        if !snapshot.descends_from(member, target) {
            let result = attribution.process(member);
            let owner = result.owner.clone();
            let allowed = !matches!(authority, Authority::Ledger)
                && authorize(member, &result, authority).is_ok();
            if !allowed {
                return Err(StopRefusal::GroupShared {
                    pid: target_pid,
                    member,
                    owner,
                });
            }
        }
        if let Some(reference) = process.reference() {
            members.push(reference);
        }
    }
    let names = members
        .iter()
        .filter_map(|m| {
            snapshot
                .processes
                .get(&m.pid)
                .map(|p| (m.pid, p.name.clone()))
        })
        .collect();
    Ok(Plan {
        group: unix_group,
        members,
        target,
        names,
    })
}
