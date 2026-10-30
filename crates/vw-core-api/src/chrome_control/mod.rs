//! Chrome Control's checked gateway, person-confirmed levels and app-owned helper. Depends on vw-ask,
//! the key service and agent tools. Called by core startup, session tools and Settings; files restore no raises.
pub(crate) mod check;
pub(crate) mod commands;
pub(crate) mod gateway;
mod mapping;
mod owner;
mod pending;
mod pipe;
mod restore;
pub(crate) mod session;
pub use restore::CHROME_RESTORE_DIALOG_METHOD;
pub(crate) use restore::start as restore_at_start;
pub(crate) mod setup;
mod store;
mod text;

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use serde_json::{Value, json};
use vw_ask::ChromeSettings;
use vw_ask::chrome::Group;

pub use commands::{ChromeControlRequest, ChromeControlView};
pub use setup::{ChromeControlConfig, ChromeSetupView};

/// Most card entries kept for the sessions' transcripts (this run only, like the ask records).
const MAX_ENTRIES: usize = 500;

/// One Chrome Control card as the session's transcript shows it. Its outcome is the ask record
/// with the same id; this holds what the card shows about the call.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ChromeAskView {
    /// The ask's id (`asks`, `askRecords`).
    pub ask_id: String,
    /// The session the card belongs to.
    pub session_id: String,
    /// The turn it happened in, when known.
    pub turn_id: Option<String>,
    /// Unix milliseconds.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub created_at_ms: u64,
    /// Vibe's tool name (`mcp_vw.chrome_execute`).
    pub tool: String,
    /// What the call does, in plain words ("Send a form or confirm on shop.example").
    pub summary: String,
    /// The group whose row decided; `None` for closing one of the user's tabs or an unknown tool.
    pub group: Option<Group>,
    /// Why Vibe asks (the card's reason line), or why it was refused.
    pub reason: String,
    /// What "Allow for this session" covers, in words; empty when the card offers no session answer.
    pub covers: Vec<vw_ask::CoversPart>,
    /// The call's input as Vibe sent it (the card's payload).
    pub input: Value,
    /// The card waits for the user's answer (the session shows Needs input); false once it closed.
    pub open: bool,
}

/// Most refused steps kept for `chrome_outcome` (this run only).
const MAX_NOT_DONE: usize = 200;
/// Refused steps one `chrome_outcome` answer lists.
const OUTCOME_LIMIT: usize = 5;

/// One Chrome Control step the guard refused, as the agent's `chrome_outcome` tells it (the hook's reason does
/// not reach the model inside `run_typescript`).
#[derive(Debug, Clone)]
struct NotDone {
    ask_id: Option<String>,
    session_id: String,
    /// What the step was ("Send a form or confirm on portal.example").
    step: String,
    /// The guard's own sentence, exactly as it denied the call.
    outcome: String,
    at_ms: u64,
}

/// Effective level and a stored raise that still needs the person's native confirmation.
struct LevelState {
    effective: ChromeSettings,
    restore: Option<ChromeSettings>,
}

/// The run's Chrome Control state (one per core).
pub(crate) struct ChromeControl {
    /// The state folder beside the Vibe home (`<app data>/chrome-control`); `None` without one (tests, no home).
    dir: Option<PathBuf>,
    settings: Mutex<LevelState>,
    entries: Mutex<VecDeque<ChromeAskView>>,
    not_done: Mutex<VecDeque<NotDone>>,
    pub(crate) config: ChromeControlConfig,
    owner: owner::Owner,
    lowered: Mutex<std::collections::BTreeMap<String, String>>,
    lifecycle: tokio::sync::Mutex<()>,
    generation: std::sync::atomic::AtomicU64,
    tasks: Mutex<std::collections::HashMap<String, String>>,
}

impl ChromeControl {
    /// The state for the Vibe home `vibe_home`: the rows are read once, here (a missing or unreadable file reads as
    /// the default, Browse). A stored raise waits at Browse until a person re-confirms it in a native dialog.
    pub(crate) fn new(_vibe_home: &Path, config: ChromeControlConfig) -> Self {
        let dir = owner::state_dir(&config);
        let saved = dir.as_deref().map(store::load).unwrap_or_default();
        let restore = ChromeSettings::default().raises(saved).then_some(saved);
        let settings = LevelState {
            effective: if restore.is_some() {
                ChromeSettings::default()
            } else {
                saved
            },
            restore,
        };
        Self {
            lowered: Mutex::new(std::collections::BTreeMap::new()),
            owner: owner::Owner::default(),
            lifecycle: tokio::sync::Mutex::new(()),
            generation: std::sync::atomic::AtomicU64::new(0),
            tasks: Mutex::new(std::collections::HashMap::new()),
            dir,
            settings: Mutex::new(settings),
            entries: Mutex::new(VecDeque::new()),
            not_done: Mutex::new(VecDeque::new()),
            config,
        }
    }

    fn current(&self) -> MutexGuard<'_, LevelState> {
        self.settings.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The level now.
    pub(crate) fn settings(&self) -> ChromeSettings {
        self.current().effective
    }

    /// The state folder, when there is one.
    pub(crate) fn dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }

    /// Saves `next` (the caller already asked for a raise) and applies it from the next call.
    pub(crate) fn save(&self, next: ChromeSettings) -> Result<(), String> {
        let mut state = self.current();
        if let Some(dir) = &self.dir {
            store::save(dir, next)?;
        }
        state.effective = next;
        state.restore = None;
        Ok(())
    }

    /// Back to the default level in memory (Remove deleted the saved one).
    pub(crate) fn reset(&self) {
        self.generation
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.tasks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
        let mut state = self.current();
        state.effective = ChromeSettings::default();
        state.restore = None;
        self.lowered
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }

    /// Remembers one refused step for the agent's `chrome_outcome`.
    pub(crate) fn not_done(&self, session_id: &str, step: &str, outcome: &str, at_ms: u64) {
        self.remember_outcome(None, session_id, step, outcome, at_ms);
    }

    fn remember_outcome(
        &self,
        ask_id: Option<&str>,
        session_id: &str,
        step: &str,
        outcome: &str,
        at_ms: u64,
    ) {
        let mut all = self.not_done.lock().unwrap_or_else(PoisonError::into_inner);
        if ask_id.is_some() && all.iter().any(|entry| entry.ask_id.as_deref() == ask_id) {
            return;
        }
        all.push_back(NotDone {
            ask_id: ask_id.map(str::to_owned),
            session_id: session_id.to_owned(),
            step: step.to_owned(),
            outcome: outcome.to_owned(),
            at_ms,
        });
        while all.len() > MAX_NOT_DONE {
            all.pop_front();
        }
    }

    /// Keeps (or replaces) one card entry.
    pub(crate) fn upsert(&self, view: ChromeAskView) {
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(at) = entries.iter().position(|e| e.ask_id == view.ask_id) {
            entries[at] = view;
            return;
        }
        entries.push_back(view);
        while entries.len() > MAX_ENTRIES {
            entries.pop_front();
        }
    }

    /// Weaves this session's card entries into a session snapshot's history, by time, like protected changes.
    pub(crate) fn overlay(&self, session_id: &str, state: &mut Value) {
        let Some(history) = state.get_mut("history").and_then(Value::as_array_mut) else {
            return;
        };
        history.retain(|entry| entry["type"] != "chrome_ask");
        let entries: Vec<ChromeAskView> = self
            .entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|e| e.session_id == session_id)
            .cloned()
            .collect();
        for view in entries {
            let next = history.iter().position(|entry| {
                entry["createdAt"]
                    .as_u64()
                    .is_some_and(|at| at > view.created_at_ms)
            });
            history.insert(next.unwrap_or(history.len()), history_entry(&view));
        }
    }
}

/// Record the answer when the card closes, independently of a later action retry.
fn record_card_outcome(shared: &crate::core::Shared, card: &ChromeAskView) {
    let Some(record) = shared.asks.outcome(&card.ask_id) else {
        return;
    };
    if matches!(record.outcome, vw_ask::Outcome::Allowed { .. }) {
        return;
    }
    let session_permission = card.tool == "mcp_vw.use_your_chrome";
    let text = vw_ask::agent_text(
        &record.outcome,
        if session_permission {
            "using your Chrome in this chat"
        } else {
            "this Chrome action"
        },
    );
    shared.chrome_control.remember_outcome(
        Some(&card.ask_id),
        &card.session_id,
        &card.summary,
        &text,
        record.closed_at,
    );
}

/// `mcp_vw.chrome_outcome` for `session_id`: this chat's refused Chrome Control steps, newest first, each with the
/// guard's own sentence (the step did not run).
pub(crate) fn outcome_tool(shared: &crate::core::Shared, session_id: &str) -> Value {
    let now = shared.asks.now_ms();
    // The answer route settles synchronously; its closing task may not have run yet.
    for card in shared
        .chrome_control
        .entries
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .filter(|c| c.session_id == session_id)
    {
        record_card_outcome(shared, card);
    }
    let mut outcomes: Vec<_> = shared
        .chrome_control
        .not_done
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .filter(|n| n.session_id == session_id)
        .cloned()
        .collect();
    outcomes.sort_by_key(|entry| entry.at_ms);
    let items: Vec<Value> = outcomes
        .iter()
        .rev()
        .take(OUTCOME_LIMIT)
        .map(|n| {
            json!({"step": n.step, "outcome": n.outcome,
                "secondsAgo": now.saturating_sub(n.at_ms) / 1000})
        })
        .collect();
    let message = if items.is_empty() {
        "No Chrome step was refused in this chat.".to_owned()
    } else {
        "These Chrome steps were not done (newest first).".to_owned()
    };
    json!({"notDone": items, "message": message})
}

/// The TypeScript declarations of Chrome Control's wire types (`gen-core-ts`, module `ask`).
#[cfg(feature = "ts")]
pub(crate) fn declarations(config: &ts_rs::Config) -> Vec<String> {
    use ts_rs::TS;
    vec![
        vw_ask::chrome::Level::decl(config),
        vw_ask::chrome::Group::decl(config),
        ChromeSettings::decl(config),
        ChromeAskView::decl(config),
        ChromeControlRequest::decl(config),
        ChromeControlView::decl(config),
        ChromeSetupView::decl(config),
        setup::SetupState::decl(config),
    ]
}

/// The history entry of one card (`type: "chrome_ask"`); the window builds the same from the event.
pub(crate) fn history_entry(view: &ChromeAskView) -> Value {
    json!({"id": format!("chrome:{}", view.ask_id), "type": "chrome_ask", "createdAt": view.created_at_ms,
        "turnId": view.turn_id, "chromeAsk": view})
}

#[cfg(test)]
mod tests;

/// Starts the installed helper without giving startup errors any secret-bearing details.
pub(crate) async fn start_owner(shared: &std::sync::Arc<crate::core::Shared>) {
    if let Err(reason) = owner::start(shared).await {
        tracing::warn!(%reason, "Chrome Control could not start");
    }
}

pub(crate) async fn stop_owner(shared: &std::sync::Arc<crate::core::Shared>) {
    shared.chrome_control.owner.stop().await;
    owner::publish(shared);
}

/// Revokes this installation's chat consent, cards and action grants.
pub(super) fn removed(shared: &crate::core::Shared) {
    if let Some(tools) = &shared.agent_tools {
        tools.on_demand.remove_chrome();
    }
    for ask in shared
        .asks
        .list()
        .into_iter()
        .filter(|ask| ask.kind.starts_with("chrome/ask/"))
    {
        shared.asks.settle(
            &ask.id,
            vw_ask::Outcome::Cancelled {
                reason: "Chrome Control was removed".to_owned(),
            },
        );
    }
    if shared
        .ask_grants
        .end_tools(|tool| tool.starts_with("chrome:"))
    {
        shared.emit(crate::ask::grants::event(shared));
    }
}
