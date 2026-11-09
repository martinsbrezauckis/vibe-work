//! The headless browser plugin loads on demand after the requesting turn ends.
//! Session reopen preserves history and waits for pending work; Stop cancels automatic continuation.
//! Called by agent tools, session lifecycle and the event pump.
use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tracing::{info, warn};
use vw_agent_tools::wire::tools as names;

use crate::core::{Core, Shared};
use crate::event::{AlertKind, CoreEvent};

/// How long the switch waits for a finished turn's session to become quiet (a queued turn, an open card).
const QUIET_WAIT: Duration = Duration::from_secs(30);

/// One tool set that loads only when needed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum ToolSet {
    /// The marketplace's headless browser plugin (Microsoft's Playwright MCP, server `browser`).
    HeadlessBrowser,
}

impl ToolSet {
    /// Every on-demand set.
    pub(crate) const ALL: [Self; 1] = [Self::HeadlessBrowser];

    /// The set a gateway tool turns on.
    pub(crate) fn of_tool(tool: &str) -> Option<Self> {
        match tool {
            names::USE_HEADLESS_BROWSER => Some(Self::HeadlessBrowser),
            _ => None,
        }
    }

    /// Vibe's tool-filter pattern for the set's tools: a `re:` full match on the display name `{server}_{tool}`,
    /// normalised (`_mcp_naming.py:137-170`). Chrome Control's server `chrome-control` shows as
    /// `chrome_control_chrome_*`; the headless browser's server `browser` (all its tools are `browser_*`) as
    /// `browser_browser_*`. Measured on Vibe 2.26.1 (`live_brw1_on_demand`).
    pub(crate) fn pattern(self) -> &'static str {
        match self {
            Self::HeadlessBrowser => "re:browser_browser_.+",
        }
    }

    /// Vibe's tool group for the set (`mcp_<server>`), as the agent finds it with `search_tool_functions`.
    fn group(self) -> &'static str {
        match self {
            Self::HeadlessBrowser => "mcp_browser",
        }
    }

    /// What the set is, in the agent's words.
    fn name(self) -> &'static str {
        match self {
            Self::HeadlessBrowser => "The headless browser's tools",
        }
    }

    /// The refusal when the set's plugin is not there.
    fn not_there(self) -> &'static str {
        match self {
            Self::HeadlessBrowser => {
                "The headless browser plugin is not installed. It is a developer tool from Tools > Marketplace; do not \
                 suggest it unless the user asks for it. Use Vibe Work's own browser."
            }
        }
    }

    /// The answer while the set turns on.
    fn turning_on(self) -> String {
        let rest = match self {
            Self::HeadlessBrowser => {
                "End this reply now with one short sentence (for example \"Starting the headless browser.\")."
            }
        };
        format!(
            "{} turn on for this chat when this reply ends, and Vibe Work then continues the chat by itself. {rest}",
            self.name()
        )
    }

    /// The answer when the set is already on.
    fn already_on(self) -> String {
        format!(
            "{} are already on in this chat: find them with `search_tool_functions` (group `{}`).",
            self.name(),
            self.group()
        )
    }

    /// One line of the continuation turn for this set.
    fn continue_line(self) -> String {
        let how = match self {
            Self::HeadlessBrowser => "Continue the user's last request with the headless browser.",
        };
        format!(
            "{} are on in this chat now (group `{}`; find them with `search_tool_functions`). {how}",
            self.name(),
            self.group()
        )
    }
}

/// A turning-on that waits for the turn to end.
#[derive(Debug, Clone, Default)]
pub(crate) struct Pending {
    sets: BTreeSet<ToolSet>,
    /// The turn that asked (the session's running turn at the call), so only its own end continues the chat.
    turn_id: Option<String>,
    /// The chat's Stop generation when that turn asked: the continuation is sent only while it is unchanged.
    generation: u64,
}

#[derive(Default)]
struct Inner {
    consented: BTreeSet<String>,
    on: HashMap<String, BTreeSet<ToolSet>>,
    pending: HashMap<String, Pending>,
    interrupts: HashMap<String, usize>,
}

/// The on-demand tool sets of every session of this run.
#[derive(Default)]
pub(crate) struct OnDemand {
    inner: Mutex<Inner>,
}

/// Keep the old channel alive until Vibe acknowledges Stop, even if its turn event arrives first.
pub(crate) struct Interruption<'a> {
    state: &'a OnDemand,
    session: String,
}

impl Drop for Interruption<'_> {
    fn drop(&mut self) {
        let mut inner = self.state.inner();
        if let Some(count) = inner.interrupts.get_mut(&self.session) {
            *count -= 1;
            if *count == 0 {
                inner.interrupts.remove(&self.session);
            }
        }
    }
}

/// What a gateway call did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Requested {
    /// The set is on in the session already.
    AlreadyOn,
    /// The set turns on when the running turn ends.
    TurningOn,
}

impl OnDemand {
    pub(crate) fn consented(&self, session: &str) -> bool {
        let inner = self.inner();
        inner.consented.contains(session)
            || inner.on.contains_key(session)
            || inner.pending.contains_key(session)
    }

    pub(crate) fn allow(&self, session: &str) {
        self.inner().consented.insert(session.to_owned());
    }
    fn inner(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn interrupting(&self, session: &str) -> Interruption<'_> {
        *self
            .inner()
            .interrupts
            .entry(session.to_owned())
            .or_default() += 1;
        Interruption {
            state: self,
            session: session.to_owned(),
        }
    }

    fn awaiting_interrupt(&self, session: &str) -> bool {
        self.inner().interrupts.contains_key(session)
    }

    /// The patterns of every set that is not on for `session` (`None`: a session that does not exist yet).
    pub(crate) fn disabled_patterns(&self, session: Option<&str>) -> Vec<&'static str> {
        let inner = self.inner();
        let on = session.and_then(|id| inner.on.get(id));
        ToolSet::ALL
            .into_iter()
            .filter(|set| !on.is_some_and(|on| on.contains(set)))
            .map(ToolSet::pattern)
            .collect()
    }

    /// Asks for `set` in `session`; `turn_id` is the turn running now, `generation` the chat's Stop generation.
    pub(crate) fn request(
        &self,
        session: &str,
        set: ToolSet,
        turn_id: Option<String>,
        generation: u64,
    ) -> Requested {
        let mut inner = self.inner();
        if inner.on.get(session).is_some_and(|on| on.contains(&set)) {
            return Requested::AlreadyOn;
        }
        let fresh = !inner.pending.contains_key(session);
        let pending = inner.pending.entry(session.to_owned()).or_default();
        pending.sets.insert(set);
        // The first ask of a waiting set, or of a later turn, names the generation the continuation needs.
        if fresh || (turn_id.is_some() && pending.turn_id != turn_id) {
            pending.generation = generation;
        }
        if turn_id.is_some() {
            pending.turn_id = turn_id;
        }
        Requested::TurningOn
    }

    fn has_pending(&self, session: &str) -> bool {
        self.inner().pending.contains_key(session)
    }

    /// Moves the session's pending sets to its on sets; `None` when nothing waits (another switch took it).
    fn take_pending(&self, session: &str) -> Option<Pending> {
        let mut inner = self.inner();
        let pending = inner.pending.remove(session)?;
        inner
            .on
            .entry(session.to_owned())
            .or_default()
            .extend(pending.sets.iter().copied());
        Some(pending)
    }

    /// Session `old` continues as `new` (`/clear`, a rewind into a new id): its sets follow it.
    pub(crate) fn hand_off(&self, old: &str, new: &str) {
        let mut inner = self.inner();
        if inner.consented.remove(old) {
            inner.consented.insert(new.to_owned());
        }
        if let Some(on) = inner.on.remove(old) {
            inner.on.insert(new.to_owned(), on);
        }
        if let Some(pending) = inner.pending.remove(old) {
            inner.pending.insert(new.to_owned(), pending);
        }
    }

    /// A deleted session's sets are dropped.
    pub(crate) fn forget(&self, session: &str) {
        let mut inner = self.inner();
        inner.consented.remove(session);
        inner.on.remove(session);
        inner.pending.remove(session);
    }
}

/// Adds the core's patterns to `config.disabledTools`, keeping any the caller sent (no duplicates).
pub(crate) fn add_patterns(config: &mut serde_json::Map<String, Value>, patterns: &[&str]) {
    if patterns.is_empty() {
        return;
    }
    let list = config
        .entry("disabledTools")
        .or_insert_with(|| Value::Array(Vec::new()));
    if !list.is_array() {
        *list = Value::Array(Vec::new());
    }
    if let Some(list) = list.as_array_mut() {
        for pattern in patterns {
            if !list.iter().any(|v| v.as_str() == Some(pattern)) {
                list.push(Value::String((*pattern).to_owned()));
            }
        }
    }
}

/// Whether the set's plugin is in the Vibe home's `plugins/` (its tools exist in a chat).
pub(crate) fn present(shared: &Shared, _set: ToolSet) -> bool {
    shared
        .instructions
        .own_home
        .join("plugins")
        .join("headless-browser")
        .is_dir()
}

/// The session's running turn, from the store.
fn running_turn(shared: &Shared, session: &str) -> Option<String> {
    let view = shared.store.view(session)?;
    view.state["turns"]
        .as_array()?
        .iter()
        .rev()
        .find(|turn| turn["status"] == "in_progress")
        .and_then(|turn| turn["id"].as_str())
        .map(str::to_owned)
}

/// The session's latest turn and its status, from the store.
fn latest_turn(shared: &Shared, session: &str) -> Option<(String, String)> {
    let view = shared.store.view(session)?;
    let turn = view.state["turns"].as_array()?.last()?;
    Some((
        turn["id"].as_str()?.to_owned(),
        turn["status"].as_str().unwrap_or_default().to_owned(),
    ))
}

/// Activates the installed headless plugin after this turn.
pub(crate) fn tool(
    shared: &Arc<Shared>,
    state: &OnDemand,
    session: &str,
    tool: &str,
) -> Result<Value, String> {
    let set = ToolSet::of_tool(tool).ok_or("unknown tool")?;
    if !present(shared, set) {
        return Err(set.not_there().to_owned());
    }
    let turn = running_turn(shared, session);
    // Bound to the turn that asks: a Stop during that turn also stops its continuation, even one asked after it.
    let generation = crate::stops::turn_generation(shared, session, turn.as_deref());
    match state.request(session, set, turn, generation) {
        Requested::AlreadyOn => Ok(json!({
            "status": "alreadyOn", "group": set.group(), "message": set.already_on(),
        })),
        Requested::TurningOn => {
            info!(
                session_id = session,
                set = ?set,
                "on-demand tools turn on when the turn ends"
            );
            Ok(json!({"status": "turningOn", "group": set.group(), "message": set.turning_on()}))
        }
    }
}

/// Chrome applies current-mode checks in the tool; headless activation has its own lifecycle.
pub(crate) fn approve_without_dialog(callback: &Value) -> bool {
    callback.pointer("/detail/kind").and_then(Value::as_str) == Some("approval")
        && callback
            .pointer("/detail/effect/toolName")
            .and_then(Value::as_str)
            .and_then(|name| name.strip_prefix("mcp_vw."))
            .is_some_and(|name| {
                ToolSet::of_tool(name).is_some() || crate::chrome_control::gateway::is_tool(name)
            })
}

/// The pump's hook: a finished turn of a session with a pending set starts the switch.
pub(crate) fn observe(shared: &Arc<Shared>, change: &vw_session::Change) {
    let session_id = match change {
        vw_session::Change::TurnUpserted { session_id, turn }
            if matches!(
                turn["status"].as_str(),
                Some("completed" | "failed" | "interrupted" | "cancelled")
            ) =>
        {
            session_id
        }
        // A marketplace request can arrive after turn/completed but before the final idle patch.
        vw_session::Change::SessionPatched { session_id, .. } if ready(shared, session_id) => {
            session_id
        }
        _ => return,
    };
    let pending_tools = shared
        .agent_tools
        .as_ref()
        .is_some_and(|tools| tools.on_demand.has_pending(session_id));
    if !pending_tools && !shared.plugin_refresh.pending(session_id) {
        return;
    }
    let (shared, id) = (shared.clone(), session_id.clone());
    tokio::spawn(async move {
        let _ = switch(shared, id, true).await;
    });
}

/// One worker for plugin changes and on-demand sets. The chat lock prevents a send crossing reload/reopen.
/// A turn-end worker waits for the final idle event; an explicit request on a busy chat only marks pending.
pub(crate) async fn switch(
    shared: Arc<Shared>,
    id: String,
    wait: bool,
) -> Result<(), crate::error::CoreError> {
    let deadline = Instant::now() + QUIET_WAIT;
    let held = loop {
        let held = shared.plugin_refresh.lock(&id).await;
        // A restart's interrupted projection can wake this turn-end worker. Its old channel died:
        // leave the switch pending for a person's next action instead of recovering the old turn.
        if wait && !shared.channels.has_channel(&id) {
            return Ok(());
        }
        if ready(&shared, &id) {
            break held;
        }
        drop(held);
        if !wait || Instant::now() >= deadline {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let pending = apply_pending(&shared, &id).await?;
    let core = Core {
        shared: shared.clone(),
    };
    drop(held);
    let Some(pending) = pending else {
        return Ok(());
    };
    let continues = latest_turn(&shared, &id).is_some_and(|(turn, status)| {
        status == "completed" && pending.turn_id.as_deref() == Some(&turn)
    });
    if !continues {
        return Ok(());
    }
    let text = pending
        .sets
        .iter()
        .map(|set| set.continue_line())
        .collect::<Vec<_>>()
        .join("\n");
    let params =
        json!({"sessionId": id, "message": [{"type": "text", "text": text}], "injected": true});
    // A Stop since the turn asked for the tools keeps this continuation from being sent (`stops.rs`).
    if let Err(error) = core
        .forward_unless_stopped("turn/start", params, pending.generation)
        .await
    {
        if crate::stops::refused_as_stopped(&error) {
            info!(
                session_id = id,
                "on-demand tools: on; the chat was stopped, so it does not continue"
            );
            return Ok(());
        }
        warn!(
            session_id = id,
            code = error.code.as_str(),
            "on-demand tools: the chat did not continue"
        );
        if error.code == crate::ErrorCode::AccessModeUnverified {
            return Ok(());
        }
        shared.emit(CoreEvent::Alert {
            session_id: Some(id),
            kind: AlertKind::ReopenFailed,
            message: "The tools are on, but the chat did not continue by itself. Send a message to go on.".to_owned(),
        });
    }
    Ok(())
}

/// Quiet with a finished last turn, or idle and never run in a chat this core started this run. Such a chat that
/// was never sent work only re-pins (`refresh_plugins_held`); one that was (or may have been) is re-pinned and
/// reattached at once, so no doubtful runtime keeps stale plugins. Any other never-run chat waits for its first
/// durable turn: closing it would lose it.
pub(crate) fn ready(shared: &Shared, id: &str) -> bool {
    let never_run_here = shared.plugin_refresh.created_here(id)
        && crate::recovery::never_ran(shared, id)
        && !crate::recovery::working(shared, id);
    (never_run_here
        || crate::recovery::quiet(shared, id)
            && latest_turn(shared, id).is_some_and(|(_, status)| status != "in_progress"))
        && !shared
            .agent_tools
            .as_ref()
            .is_some_and(|tools| tools.on_demand.awaiting_interrupt(id))
}

/// Caller holds the chat lock. Taking both kinds of pending work guarantees one reopen.
/// A user send consumes the switch without an injected continuation: that message continues the chat.
pub(crate) async fn apply_pending(
    shared: &Arc<Shared>,
    id: &str,
) -> Result<Option<Pending>, crate::error::CoreError> {
    let plugins = shared.plugin_refresh.take(id);
    // An on-demand set needs a reopen, which would discard a chat that never ran: it waits for the first turn.
    let pending = if shared.plugin_refresh.unstarted(shared, id) {
        None
    } else {
        shared
            .agent_tools
            .as_ref()
            .and_then(|tools| tools.on_demand.take_pending(id))
    };
    if !plugins && pending.is_none() {
        return Ok(None);
    }
    let core = Core {
        shared: shared.clone(),
    };
    let refreshed = if plugins {
        core.refresh_plugins_held(id).await
    } else {
        core.reopen_tools(id).await
    };
    // Even a failed plugin reload must not strand an independently requested Chrome switch.
    if let Err(error) = refreshed {
        // Fail closed: the change stays pending, so every later call of this chat that can build or use its runtime
        // (turn, queue, shell command, context) retries the re-pin first and is refused while it fails (a removed or
        // turned-off plugin never runs again).
        if plugins {
            shared.plugin_refresh.request(id);
        }
        if error.code == crate::ErrorCode::ServerError && pending.is_some() {
            core.reopen_tools(id).await?;
        }
        return Err(error);
    }
    Ok(pending)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headless_activation_does_not_filter_chrome() {
        let state = OnDemand::default();
        assert_eq!(state.disabled_patterns(None), ["re:browser_browser_.+"]);
        assert_eq!(ToolSet::of_tool("use_your_chrome"), None);
        assert_eq!(
            state.request("a", ToolSet::HeadlessBrowser, Some("t1".into()), 0),
            Requested::TurningOn
        );
        state.hand_off("a", "b");
        assert!(state.has_pending("b"));
        assert_eq!(
            state.take_pending("b").unwrap().turn_id.as_deref(),
            Some("t1")
        );
        assert_eq!(state.disabled_patterns(Some("b")), [] as [&str; 0]);
        assert_eq!(state.disabled_patterns(Some("a")).len(), 1);
        state.forget("b");
        assert_eq!(state.disabled_patterns(Some("b")).len(), 1);
    }

    #[test]
    fn the_core_patterns_join_the_callers_own_without_duplicates() {
        let mut config = serde_json::Map::new();
        config.insert(
            "disabledTools".into(),
            json!(["bash", "re:vw_chrome_(status|session|execute)"]),
        );
        add_patterns(
            &mut config,
            &[
                "re:vw_chrome_(status|session|execute)",
                "re:browser_browser_.+",
            ],
        );
        assert_eq!(
            config["disabledTools"],
            json!([
                "bash",
                "re:vw_chrome_(status|session|execute)",
                "re:browser_browser_.+"
            ])
        );
        let mut empty = serde_json::Map::new();
        add_patterns(&mut empty, &[]);
        assert!(
            empty.get("disabledTools").is_none(),
            "nothing added when every set is on"
        );
    }

    #[test]
    fn only_the_turning_on_calls_skip_vibes_card() {
        let card =
            |tool: &str| json!({"detail": {"kind": "approval", "effect": {"toolName": tool}}});
        assert!(!approve_without_dialog(&card("mcp_vw.use_your_chrome")));
        assert!(approve_without_dialog(&card("mcp_vw.use_headless_browser")));
        assert!(!approve_without_dialog(&card("mcp_vw.process_start")));
        assert!(!approve_without_dialog(&card(
            "mcp_chrome_control.chrome_page_commit"
        )));
        assert!(!approve_without_dialog(&card("use_your_chrome")));
        let question = json!({"detail": {"kind": "question", "effect": {"toolName": "mcp_vw.use_your_chrome"}}});
        assert!(!approve_without_dialog(&question));
    }
}
