//! Serializable read views of core state: session snapshots, anomaly reports, open callbacks,
//! supervisor status and the Debug API flag.
//!
//! Depends on: vw-session (views and anomalies), vw-supervisor (state), vw-gate (open callbacks),
//! vw-rpc (death reports). Called by: `command` (results), `event` (stream items), `dispatch`.
//! Invariants:
//! - Pure copies: building a view never changes state.
//! - Nothing here carries a secret. Stderr lines come from vw-rpc's ring, where the API key is
//!   already replaced (`ConnectionConfig::stderr_redactions`), and death reports keep only the
//!   cause, exit status and that redacted tail.
//! - Integer counters are `u64` in Rust and `number` in TypeScript (safe below 2^53).

use crate::keys::RateLimitsView;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use vw_gate::OpenCallback;
use vw_rpc::DeathReport;
use vw_session::{AnomalyCounts, AnomalyKind, AnomalyRecord, SessionRow, SessionView};
use vw_supervisor::SupervisorState;

/// One session as the store holds it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SessionSnapshot {
    /// Session id.
    pub session_id: String,
    /// The full `PublicSessionState` as wire JSON.
    pub state: Value,
    /// The resolved access mode for display, including the app's trusted record before attachment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub access_mode: Option<String>,
    /// Highest event id applied.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub last_event_id: u64,
    /// We created it and the server's listing does not show it yet.
    pub local_only: bool,
    /// A gap or failed patch is waiting for a re-read.
    pub resync_pending: bool,
    /// Turns this store marked interrupted after a resume.
    pub interrupted_turns: Vec<String>,
    /// Anomaly counters.
    pub anomalies: AnomalyCountsView,
}

impl From<SessionView> for SessionSnapshot {
    fn from(v: SessionView) -> Self {
        Self {
            session_id: v.session_id,
            state: v.state,
            access_mode: None,
            last_event_id: v.last_event_id,
            local_only: v.local_only,
            resync_pending: v.resync_pending,
            interrupted_turns: v.interrupted_turns,
            anomalies: v.anomalies.into(),
        }
    }
}

/// Counters per anomaly kind (see `vw_session::AnomalyKind`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct AnomalyCountsView {
    /// Repeated event ids (benign after a reload).
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub duplicate_events: u64,
    /// Event ids that skipped ahead (a resync followed).
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub event_gaps: u64,
    /// History entries announced again and upserted.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub reannounced_entries: u64,
    /// Patches to entries already completed.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub completed_entry_updates: u64,
    /// Patches that could not be applied (a resync followed).
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub patch_failures: u64,
    /// Failed patches covered by a clean session read; excluded from significant anomalies.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub recovered_patch_failures: u64,
    /// Notifications missing a field the reducer needs.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub malformed_payloads: u64,
    /// Snapshots that disagreed with their own envelope.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub inconsistent_envelopes: u64,
    /// Turns marked interrupted locally after a resume.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub interrupted_turns: u64,
}

impl From<AnomalyCounts> for AnomalyCountsView {
    fn from(c: AnomalyCounts) -> Self {
        Self {
            duplicate_events: c.duplicate_events,
            event_gaps: c.event_gaps,
            reannounced_entries: c.reannounced_entries,
            completed_entry_updates: c.completed_entry_updates,
            patch_failures: c.patch_failures,
            recovered_patch_failures: c.recovered_patch_failures,
            malformed_payloads: c.malformed_payloads,
            inconsistent_envelopes: c.inconsistent_envelopes,
            interrupted_turns: c.interrupted_turns,
        }
    }
}

/// One remembered anomaly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct AnomalyRecordView {
    /// Kind, in camelCase (`eventGap`, `patchFailed`, ...).
    pub kind: String,
    /// Event id involved.
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub event_id: Option<u64>,
    /// Description.
    pub detail: String,
}

fn kind_name(kind: AnomalyKind) -> &'static str {
    match kind {
        AnomalyKind::DuplicateEvent => "duplicateEvent",
        AnomalyKind::EventGap => "eventGap",
        AnomalyKind::ReannouncedEntry => "reannouncedEntry",
        AnomalyKind::CompletedEntryUpdated => "completedEntryUpdated",
        AnomalyKind::PatchFailed => "patchFailed",
        AnomalyKind::PatchRecovered => "patchRecovered",
        AnomalyKind::MalformedPayload => "malformedPayload",
        AnomalyKind::InconsistentEnvelope => "inconsistentEnvelope",
        AnomalyKind::InterruptedTurn => "interruptedTurn",
    }
}

impl From<AnomalyRecord> for AnomalyRecordView {
    fn from(r: AnomalyRecord) -> Self {
        Self {
            kind: kind_name(r.kind).to_owned(),
            event_id: r.event_id,
            detail: r.detail,
        }
    }
}

/// Counters plus the most recent non-duplicate records, oldest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct AnomalyReport {
    /// Session id.
    pub session_id: String,
    /// Counters.
    pub counts: AnomalyCountsView,
    /// Sum of the counters except benign duplicates.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub significant: u64,
    /// Recent records.
    pub recent: Vec<AnomalyRecordView>,
}

/// One row of the merged session list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SessionRowView {
    /// The `PublicSession` object.
    pub session: Value,
    /// The resolved access mode for display before the session has a runtime.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub access_mode: Option<String>,
    /// The server's listing does not contain it yet.
    pub local_only: bool,
}

impl From<SessionRow> for SessionRowView {
    fn from(r: SessionRow) -> Self {
        Self {
            session: r.session,
            access_mode: None,
            local_only: r.local_only,
        }
    }
}

/// A callback waiting for an answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct CallbackInfo {
    /// Callback id.
    pub callback_id: String,
    /// Session it belongs to.
    pub session_id: String,
    /// `approval`, `user_input`, `connector_auth` or a newer kind.
    pub kind: String,
}

impl From<OpenCallback> for CallbackInfo {
    fn from(c: OpenCallback) -> Self {
        Self {
            callback_id: c.id,
            session_id: c.session_id,
            kind: c.kind.as_str().to_owned(),
        }
    }
}

/// Why and whether the Debug API is on; the UI shows "Debug API on" while `enabled`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct DebugApiFlag {
    /// The server is listening.
    pub enabled: bool,
    /// `dev-build`, `user-setting` or `off`.
    pub reason: String,
}

/// How a dead server ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct DeathView {
    /// Stable cause name (`eof`, `wedged`, ...).
    pub cause: String,
    /// Human-readable cause (no server output).
    pub detail: String,
    /// Exit code, when known.
    pub exit_code: Option<i32>,
    /// Terminating signal on Unix, when known.
    pub signal: Option<i32>,
}

impl From<&DeathReport> for DeathView {
    fn from(r: &DeathReport) -> Self {
        Self {
            cause: r.cause.kind().to_owned(),
            detail: r.cause.to_string(),
            exit_code: r.exit.and_then(|e| e.code),
            signal: r.exit.and_then(|e| e.signal),
        }
    }
}

/// A session whose resume is in progress.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct RecoveringView {
    /// Session id.
    pub session_id: String,
    /// Milliseconds since the resume was sent.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub elapsed_ms: u64,
    /// Expected wait (about 31 s, measured).
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub expected_ms: u64,
}

/// The supervisor's state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ServerStatus {
    /// First start in progress.
    Starting {},
    /// Vibe is not installed yet: the setup screen offers the install; nothing runs.
    NotInstalled {
        /// Why, in plain words.
        reason: String,
    },
    /// A server is up. Sessions in `recovering` are being resumed.
    Running {
        /// Starts so far.
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        generation: u64,
        /// Server name from `initialize`.
        server_name: String,
        /// Server (Vibe) version.
        server_version: String,
        /// Sessions that still need `session/resume` after a restart.
        needs_resume: Vec<String>,
        /// Resumes in progress.
        recovering: Vec<RecoveringView>,
    },
    /// The server died; a restart is scheduled.
    Restarting {
        /// Attempt number.
        attempt: u32,
        /// Delay before the attempt, in milliseconds.
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        retry_in_ms: u64,
        /// How it died.
        last_death: DeathView,
    },
    /// Shut down on request.
    Stopped {
        /// Exit code of the last server.
        exit_code: Option<i32>,
    },
    /// Gave up restarting.
    Dead {
        /// Restarts tried.
        attempts: u32,
        /// Last start error (no server output).
        reason: String,
        /// How the last server died.
        last_death: Option<DeathView>,
    },
}

fn millis(d: std::time::Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

impl From<&SupervisorState> for ServerStatus {
    fn from(s: &SupervisorState) -> Self {
        match s {
            SupervisorState::Starting => Self::Starting {},
            SupervisorState::NotInstalled { reason } => Self::NotInstalled {
                reason: reason.clone(),
            },
            SupervisorState::Ready {
                generation,
                server,
                needs_resume,
                recovering,
            } => Self::Running {
                generation: *generation,
                server_name: server.name.clone(),
                server_version: server.version.clone(),
                needs_resume: needs_resume.clone(),
                recovering: recovering
                    .iter()
                    .map(|r| RecoveringView {
                        session_id: r.session_id.clone(),
                        elapsed_ms: millis(r.elapsed),
                        expected_ms: millis(r.expected),
                    })
                    .collect(),
            },
            SupervisorState::Restarting {
                attempt,
                retry_in,
                last_death,
            } => Self::Restarting {
                attempt: *attempt,
                retry_in_ms: millis(*retry_in),
                last_death: last_death.into(),
            },
            SupervisorState::Stopped { exit } => Self::Stopped {
                exit_code: exit.and_then(|e| e.code),
            },
            SupervisorState::Failed {
                attempts,
                last_death,
                reason,
            } => Self::Dead {
                attempts: *attempts,
                reason: reason.clone(),
                last_death: last_death.as_ref().map(DeathView::from),
            },
        }
    }
}

/// The supervisor's state plus diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ServerReport {
    /// State.
    pub status: ServerStatus,
    /// OS process id of the server leader, when one runs.
    pub pid: Option<u32>,
    /// Last stderr lines (oldest first), the API key replaced.
    pub stderr_tail: Vec<String>,
}

/// Everything `readStatus` reports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct CoreStatus {
    /// Vibe Work version.
    pub app_version: String,
    /// Vibe (app-server) version of the running server, when known.
    pub vibe_version: Option<String>,
    /// Milliseconds since the core started.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub uptime_ms: u64,
    /// The supervisor.
    pub server: ServerReport,
    /// The Debug API flag.
    pub debug_api: DebugApiFlag,
    /// Sessions loaded in the store.
    pub loaded_sessions: Vec<String>,
    /// The session channels of the multiplexed server.
    pub channels: ChannelsReport,
    /// The latest per-minute rate-limit headroom a Mistral answer reported through the model proxy
    /// ; `null` until one did, or without a key service.
    pub rate_limits: Option<RateLimitsView>,
}

/// The session channels and their counters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ChannelsReport {
    /// Open channels, by channel id.
    pub open: Vec<ChannelView>,
    /// Events or callbacks dropped because they named a session another channel carries (cross-talk;
    /// never seen from Vibe, must stay 0).
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub foreign_dropped: u64,
    /// `session/read` resyncs the core started on its own since it started (gaps, lag, failed
    /// patches, restarts). Running sessions need none: their channels stream every event.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub resync_reads: u64,
    /// Sessions Vibe could not reopen after a restart (their commands are refused).
    pub stuck: Vec<StuckView>,
}

/// One stuck session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct StuckView {
    /// The stuck session.
    pub session_id: String,
    /// The session `continueInNewSession` started for it, when there is one.
    pub continued_in: Option<String>,
}

/// One open session channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ChannelView {
    /// The session it carries.
    pub session_id: String,
    /// The channel id on the current server process.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub channel_id: u64,
    /// Milliseconds since the channel was last used (a request or an event).
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub idle_ms: u64,
}

pub(crate) fn duration_ms(d: std::time::Duration) -> u64 {
    millis(d)
}
