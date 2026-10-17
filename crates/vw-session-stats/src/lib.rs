//! Statistics from Vibe 2.26.1's stored metadata and message stream.
//!
//! Uses serde and standard readers; called by core clients without an app or UI dependency.
//! Measurements absent from the stored format stay unknown. Turn callbacks avoid retaining history.

mod prices;
mod scan;
mod stored;
mod types;

pub use prices::{ModelPrices, PriceTable};
pub use types::{
    Error, LastModelCallSnapshot, ModelSnapshot, ParentTurn, RecordedToolOutcomes, SessionStats,
    SubagentStats, TokenUsage, ToolCounts, ToolUsage, TurnStats,
};

use std::io::Read;
use std::path::Path;

/// Scan a stored session and its linked subagent sessions, emitting turns as they are read.
///
/// A turn begins at a non-injected user message without manual-shell context. Child turns are
/// emitted with their parent turn and tool-call ID before the containing parent turn. Session
/// counters stay separate from child counters; `attributed_usage` includes both exactly once.
/// Only the single subagent depth supported by Vibe is read. Missing child paths/files are unknown.
///
/// Memory holds selected metadata, tool kinds, the current turn and outstanding tool calls;
/// completed turns and message content are never retained. The caller controls callback storage.
/// Read a quiescent session: these files do not provide a transaction across metadata and messages.
///
/// # Errors
/// Returns IO/JSON errors for unreadable or malformed files, or invalid recorded measurements,
/// child identities, child paths or ambiguous tool-call IDs. Previously emitted turns are provisional
/// until this function succeeds; discard them on error.
pub fn scan_session(
    directory: &Path,
    prices: Option<&PriceTable>,
    on_turn: impl FnMut(TurnStats),
) -> Result<SessionStats, Error> {
    scan::directory(directory, prices, on_turn)
}

/// Scan caller-owned metadata and message readers without opening any files.
///
/// Uses the same streaming parser as [`scan_session`]. Linked child files are unavailable here;
/// task results still report their recorded `turns_used` and `completed` fields. The caller can
/// subsequently scan each child separately using the emitted parent attribution.
///
/// # Errors
/// Returns IO/JSON errors or invalid recorded measurements/ambiguous tool-call IDs.
pub fn scan_readers(
    metadata: impl Read,
    messages: impl Read,
    prices: Option<&PriceTable>,
    on_turn: impl FnMut(TurnStats),
) -> Result<SessionStats, Error> {
    scan::readers(metadata, messages, prices, on_turn)
}
