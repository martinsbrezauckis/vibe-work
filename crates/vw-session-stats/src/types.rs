//! Public statistics and errors; serde serializes these for callers, with null for unknown fields.
//! The scanner populates recorded counters separately from transcript counts and configuration snapshots.

use serde::Serialize;
use std::collections::BTreeMap;

/// Failure to read a session or use an invalid measurement.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A session file could not be read.
    #[error("session file could not be read: {0}")]
    Io(#[from] std::io::Error),
    /// The stored JSON is malformed or has an invalid field type.
    #[error("invalid stored session JSON: {0}")]
    Json(#[from] serde_json::Error),
    /// Recorded cached tokens exceed recorded input tokens.
    #[error("cached tokens exceed input tokens")]
    InvalidUsage,
    /// A recorded duration is negative or non-finite.
    #[error("recorded duration must be finite and nonnegative")]
    InvalidDuration,
    /// Caller-supplied rates are negative or non-finite.
    #[error("prices must be finite and nonnegative")]
    InvalidPrice,
    /// A recorded child path is not a descendant path.
    #[error("child session path must be relative and stay within the parent directory")]
    InvalidChildPath,
    /// Child metadata disagrees with its parent link.
    #[error("child session identity disagrees with its recorded parent link")]
    ChildIdentityMismatch,
    /// Tool-call IDs within an outstanding batch are ambiguous.
    #[error("duplicate outstanding tool-call ID")]
    DuplicateToolCallId,
    /// Metadata contains multiple child links for one task call.
    #[error("duplicate child-session tool-call link")]
    DuplicateChildLink,
    /// A numeric aggregation overflowed.
    #[error("session statistic overflow")]
    Overflow,
}

/// Recorded token counts. Input includes the cached subset; absent counters stay unknown.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct TokenUsage {
    /// Prompt tokens as recorded by Vibe.
    pub input: Option<u64>,
    /// Completion tokens as recorded by Vibe.
    pub output: Option<u64>,
    /// Cached prompt tokens as recorded by Vibe.
    pub cached: Option<u64>,
}

impl TokenUsage {
    pub(crate) fn validate(self) -> Result<Self, Error> {
        if matches!((self.input, self.cached), (Some(input), Some(cached)) if cached > input) {
            return Err(Error::InvalidUsage);
        }
        Ok(self)
    }

    pub(crate) fn add(self, other: Self) -> Result<Self, Error> {
        fn sum(left: Option<u64>, right: Option<u64>) -> Result<Option<u64>, Error> {
            left.zip(right)
                .map(|(a, b)| a.checked_add(b).ok_or(Error::Overflow))
                .transpose()
        }
        Ok(Self {
            input: sum(self.input, other.input)?,
            output: sum(self.output, other.output)?,
            cached: sum(self.cached, other.cached)?,
        })
    }
}

/// The latest saved configuration, not evidence of the model used by historical turns.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ModelSnapshot {
    /// Active model alias from the config snapshot.
    pub alias: Option<String>,
    /// Model name if the snapshot contains a matching model definition.
    pub name: Option<String>,
    /// Provider if the matching model definition records it.
    pub provider: Option<String>,
}

/// Vibe's misleadingly named `last_turn_*` fields describe the latest successful model request.
/// They have no message/turn ID and may be stale after a failure or reset to zero by a reload.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct LastModelCallSnapshot {
    /// The latest recorded model-request token counters, without turn attribution.
    pub usage: TokenUsage,
    /// The latest recorded model-request duration, without turn attribution.
    pub duration_seconds: Option<f64>,
}

/// Vibe's cumulative tool-outcome counters, which can outlive retained transcript messages.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct RecordedToolOutcomes {
    /// Vibe's cumulative agreed counter.
    pub agreed: Option<u64>,
    /// Vibe's cumulative rejected counter.
    pub rejected: Option<u64>,
    /// Vibe's cumulative hook-denied counter.
    pub hook_denied: Option<u64>,
    /// Vibe's cumulative failed counter.
    pub failed: Option<u64>,
    /// Vibe's cumulative succeeded counter, which may include returned cancelled results.
    pub succeeded: Option<u64>,
}

/// Recorded tool requests and matching response messages in the retained transcript.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ToolCounts {
    /// Number of requested calls; includes calls denied, cancelled or never run.
    pub calls: u64,
    /// Matching tool response messages; a response alone does not prove success.
    pub results: u64,
    /// Calls whose response records a duration.
    pub timed_calls: u64,
    /// Responses explicitly recording cancellation.
    pub cancelled: u64,
    /// Sum of recorded durations only; partial when `timed_calls < calls`.
    pub recorded_seconds: f64,
}

impl ToolCounts {
    /// Complete sum of call durations, or unknown when any requested call lacks a duration.
    /// This is summed execution time, not elapsed wall time; concurrent calls may overlap.
    #[must_use]
    pub fn duration_seconds(&self) -> Option<f64> {
        (self.timed_calls == self.calls).then_some(self.recorded_seconds)
    }

    pub(crate) fn merge(&mut self, other: &Self) -> Result<(), Error> {
        self.calls = self.calls.checked_add(other.calls).ok_or(Error::Overflow)?;
        self.results = self
            .results
            .checked_add(other.results)
            .ok_or(Error::Overflow)?;
        self.timed_calls = self
            .timed_calls
            .checked_add(other.timed_calls)
            .ok_or(Error::Overflow)?;
        self.cancelled = self
            .cancelled
            .checked_add(other.cancelled)
            .ok_or(Error::Overflow)?;
        self.recorded_seconds += other.recorded_seconds;
        if !self.recorded_seconds.is_finite() {
            return Err(Error::Overflow);
        }
        Ok(())
    }
}

/// Tool statistics grouped by recorded function name, including unnamed calls explicitly.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ToolUsage {
    /// All requested tools in this scope.
    pub total: ToolCounts,
    /// Tools grouped by name; names are not mapped to a built-in catalogue.
    pub by_kind: BTreeMap<String, ToolCounts>,
    /// Calls without a recorded function name.
    pub unnamed: ToolCounts,
    /// Tool responses without an outstanding matching ID; never guessed from their name/content.
    pub unmatched_results: u64,
}

impl ToolUsage {
    pub(crate) fn record(&mut self, name: Option<&str>, counts: &ToolCounts) -> Result<(), Error> {
        self.total.merge(counts)?;
        match name {
            Some(name) => self
                .by_kind
                .entry(name.to_owned())
                .or_default()
                .merge(counts)?,
            None => self.unnamed.merge(counts)?,
        }
        Ok(())
    }
}

/// Proven parent attribution from `child_sessions[].tool_call_id` and the containing user turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ParentTurn {
    /// Parent session ID.
    pub session_id: String,
    /// One-based retained user-turn index, unknown when the call precedes a retained user boundary.
    pub turn_index: Option<u64>,
    /// Parent's recorded task call ID.
    pub tool_call_id: String,
}

/// One linked task's recorded outcome and optional child session summary.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SubagentStats {
    /// Child session ID from the metadata link.
    pub session_id: String,
    /// Tool call that launched the child.
    pub tool_call_id: String,
    /// Recorded agent profile name.
    pub agent: String,
    /// Recorded path beneath the parent session, if present.
    pub relative_path: Option<String>,
    /// Task output's model-iteration count, which differs from retained user turns.
    pub turns_used: Option<u64>,
    /// Task output's completion flag; does not determine parent turn success.
    pub completed: Option<bool>,
    /// Child summary; unknown without a readable linked child file pair.
    pub session: Option<SessionStats>,
}

/// A retained user turn. Vibe 2.26.1 does not persist its usage, model, timing or outcome.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TurnStats {
    /// Session owning this turn.
    pub session_id: String,
    /// One-based retained user-turn index, not Vibe's model-iteration `steps` counter.
    pub index: u64,
    /// Recorded user message ID, if present.
    pub message_id: Option<String>,
    /// Attribution for a child turn emitted while scanning a parent session.
    pub parent: Option<ParentTurn>,
    /// Model used; unknown in this stored format.
    pub model: Option<String>,
    /// Token counts for the whole user turn; unknown in this stored format.
    pub usage: TokenUsage,
    /// Turn wall time; unknown in this stored format.
    pub wall_seconds: Option<f64>,
    /// Sum of tool execution durations, unknown if any requested call lacks one.
    pub tool_seconds: Option<f64>,
    /// Total model time for this user turn; unknown in this stored format.
    pub model_seconds: Option<f64>,
    /// Turn success; unknown even if the retained assistant text appears complete.
    pub succeeded: Option<bool>,
    /// Cost in the caller's currency; unknown without attributed usage and prices.
    pub cost: Option<f64>,
    /// Local tool calls; child tool calls stay in their child's summary.
    pub tools: ToolUsage,
    /// Tasks launched during this turn, with recorded child totals if available.
    pub subagents: Vec<SubagentStats>,
}

/// Session counters and retained transcript counts; no model distribution is inferred.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SessionStats {
    /// Recorded session ID.
    pub session_id: String,
    /// Parent session ID in metadata, if present.
    pub parent_session_id: Option<String>,
    /// Recorded creation timestamp; not a turn start.
    pub start_time: Option<String>,
    /// Latest save timestamp; not evidence of a completed session or turn.
    pub last_saved_at: Option<String>,
    /// Model used throughout the session; unknown without model history.
    pub model: Option<String>,
    /// Latest config snapshot; never used to assign past usage to a model.
    pub model_snapshot: ModelSnapshot,
    /// Local cumulative usage counters, not sums reconstructed from the retained transcript.
    pub usage: TokenUsage,
    /// Latest model-call snapshot, without an inferred turn association.
    pub last_model_call: LastModelCallSnapshot,
    /// Vibe's recorded step counter; model iterations, not user turns.
    pub recorded_steps: Option<u64>,
    /// Cumulative outcome counters exactly as saved, without assigning them to retained turns.
    pub recorded_tools: RecordedToolOutcomes,
    /// Retained user turns emitted by this scan, excluding child turns.
    pub turns: u64,
    /// Retained local message count, including injected and manual-shell messages.
    pub messages: u64,
    /// Local tool-call counts and durations from the retained transcript.
    pub tools: ToolUsage,
    /// Tool calls before any retained user-turn boundary, such as a compacted prefix.
    pub unattributed_tool_calls: u64,
    /// Summed tool duration in the retained local transcript, unknown if any call lacks a duration.
    pub tool_seconds: Option<f64>,
    /// Total session model time; unknown in this stored format.
    pub model_seconds: Option<f64>,
    /// Session wall time; unknown in this stored format.
    pub wall_seconds: Option<f64>,
    /// Local cost; only a caller's explicit uniform rate can price unassigned model usage.
    pub cost: Option<f64>,
    /// Number of metadata child links encountered in retained tool calls.
    pub subagent_sessions: u64,
    /// Sum of local and encountered child usage, unknown where any contributing counter is missing.
    pub attributed_usage: TokenUsage,
    /// Local plus child cost; unknown when any contributing cost is unknown.
    pub attributed_cost: Option<f64>,
}
