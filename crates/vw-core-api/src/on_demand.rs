//! Tools that load only when needed.
//!
//! - **Off when a chat opens.** Every session-opening request the core sends gets `agentConfig.disabledTools` with the
//!   pattern of each on-demand tool set that is not on for that session ([`OnDemand::disabled_patterns`], added by
//!   `agent_tools::AgentTools::inject` after the gate, next to our MCP server). Vibe 2.26.1 filters the session's MCP
//!   tools, plugin servers included, by these patterns (`_mcp_naming.py:55`, display name `{server}_{tool}`).
//! - **On after the gateway call.** `use_your_chrome` / `use_headless_browser` (our MCP server) mark the session
//!   ([`tool`]). Vibe cannot change a running turn's tools (`_session.py:857-866`), so the switch waits for the turn
//!   to end: the pump calls [`observe`] for every finished turn; once the session is quiet the core closes its channel
//!   and reopens it (the ordinary idle close and reopen: the resume carries the new filter), then, when the turn that
//!   asked completed and is still the latest, sends one injected turn (`injected: true`, not shown in the chat) that
//!   continues the chat. A stopped or failed turn only switches; the user's next message has the tools.
//! - **First use of Chrome asks.** `chrome_control::session` opens the one `AskCard` before Chrome is enabled in a
//!   chat. The existing pre-tool Chrome guard waits outside Vibe's short MCP timeout; a direct gateway call enforces
//!   the same check. After Allow, every Chrome operation still passes the guard and the user's level. Vibe's own
//!   approval for the gateway is answered automatically (`approve_without_dialog`), avoiding a second card.
//! - **Lifetime.** Memory only, per session, for this run: an idle close and reopen keeps it, a hand-off moves it, a
//!   deleted session drops it, an app restart forgets it (the agent asks again, one step).
//!
//! Depends on: `core::Shared` (store, channels, Chrome Control's setup view, the Vibe home), `recovery::quiet`,
//! `Core::attach_session` / `Core::forward`. Called by: `agent_tools` (inject, the tool, hand-off, delete), `pump`.
use std::collections::{BTreeSet, HashMap, HashSet};
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
    /// Chrome Control (the user's own Chrome).
    YourChrome,
    /// The marketplace's headless browser plugin (Microsoft's Playwright MCP, server `browser`).
    HeadlessBrowser,
}

impl ToolSet {
    /// Every on-demand set.
    pub(crate) const ALL: [Self; 2] = [Self::YourChrome, Self::HeadlessBrowser];

    /// The set a gateway tool turns on.
    pub(crate) fn of_tool(tool: &str) -> Option<Self> {
        match tool {
            names::USE_YOUR_CHROME => Some(Self::YourChrome),
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
            Self::YourChrome => "re:vw_chrome_(status|task|execute)",
            Self::HeadlessBrowser => "re:browser_browser_.+",
        }
    }

    /// Vibe's tool group for the set (`mcp_<server>`), as the agent finds it with `search_tool_functions`.
    fn group(self) -> &'static str {
        match self {
            Self::YourChrome => "mcp_vw",
            Self::HeadlessBrowser => "mcp_browser",
        }
    }

    /// What the set is, in the agent's words.
    fn name(self) -> &'static str {
        match self {
            Self::YourChrome => "Chrome Control's tools",
            Self::HeadlessBrowser => "The headless browser's tools",
        }
    }

    /// The refusal when the set's plugin is not there.
    fn not_there(self) -> &'static str {
        match self {
            Self::YourChrome => {
                "Chrome Control is not set up in Vibe Work. Tell the user they can set it up in Settings › Browser › \
                 Your Chrome, then ask again. Until then use Vibe Work's own browser only if the user agrees."
            }
            Self::HeadlessBrowser => {
                "The headless browser plugin is not installed. It is a developer tool from Tools > Marketplace; do not \
                 suggest it unless the user asks for it. Use Vibe Work's own browser."
            }
        }
    }

    /// The answer while the set turns on.
    fn turning_on(self) -> String {
        let rest = match self {
            Self::YourChrome => {
                "End this reply now with one short sentence (for example \"Opening your Chrome.\"). Do not use Vibe \
                 Work's own browser for this task. Every Chrome step still follows the user's Chrome settings in Vibe \
                 Work; a refused step comes back \"Tool execution was skipped by Runtime policy\": then call \
                 `tools.mcp_vw.chrome_outcome({})` for the user's answer."
            }
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
            Self::YourChrome => {
                "Continue the user's last request in their own Chrome. Every Chrome step follows the user's Chrome \
                 settings. A Chrome step that comes back \"Tool execution was skipped by Runtime policy\" was not \
                 done: the user refused it in Vibe Work; call \
                 `tools.mcp_vw.chrome_outcome({})` for the user's answer and tell the user."
            }
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
    on: HashMap<String, BTreeSet<ToolSet>>,
    pending: HashMap<String, Pending>,
    interrupts: HashMap<String, usize>,
    chrome_allowed: HashSet<String>,
    chrome_waits: HashMap<String, Arc<tokio::sync::Mutex<()>>>,
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

    /// Serializes the first-use card for this chat, including concurrent agent calls.
    pub(crate) fn chrome_wait(&self, session: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.inner()
            .chrome_waits
            .entry(session.to_owned())
            .or_default()
            .clone()
    }

    /// Whether the person has allowed Chrome in this chat during this run.
    pub(crate) fn chrome_allowed(&self, session: &str) -> bool {
        self.inner().chrome_allowed.contains(session)
    }

    /// Remembers only the session answer; this changes no Chrome level or operation grant.
    pub(crate) fn allow_chrome(&self, session: &str) {
        self.inner().chrome_allowed.insert(session.to_owned());
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
        if inner.chrome_allowed.remove(old) {
            inner.chrome_allowed.insert(new.to_owned());
        }
        if let Some(wait) = inner.chrome_waits.remove(old) {
            inner.chrome_waits.insert(new.to_owned(), wait);
        }
        if let Some(on) = inner.on.remove(old) {
            inner.on.insert(new.to_owned(), on);
        }
        if let Some(pending) = inner.pending.remove(old) {
            inner.pending.insert(new.to_owned(), pending);
        }
    }

    /// Remove revokes Chrome in every chat without changing other on-demand tools.
    pub(crate) fn remove_chrome(&self) {
        let mut inner = self.inner();
        inner.chrome_allowed.clear();
        for on in inner.on.values_mut() {
            on.remove(&ToolSet::YourChrome);
        }
        inner.pending.retain(|_, pending| {
            pending.sets.remove(&ToolSet::YourChrome);
            !pending.sets.is_empty()
        });
    }

    /// A deleted session's sets are dropped.
    pub(crate) fn forget(&self, session: &str) {
        let mut inner = self.inner();
        inner.on.remove(session);
        inner.pending.remove(session);
        inner.chrome_allowed.remove(session);
        inner.chrome_waits.remove(session);
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
pub(crate) fn present(shared: &Shared, set: ToolSet) -> bool {
    if set == ToolSet::YourChrome {
        return shared
            .chrome_control
            .dir()
            .is_some_and(|dir| dir.join("setup.json").is_file());
    }
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

/// `use_your_chrome` / `use_headless_browser` for `session`. Setup refusals are plain sentences; Chrome's session
/// answer is a normal result and refusals are also kept for `chrome_outcome`.
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

/// Vibe's own gateway approval is redundant: Chrome's session `AskCard` already decided in the pre-tool guard.
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
    fn a_new_session_gets_every_pattern_and_an_on_set_drops_only_its_own() {
        let state = OnDemand::default();
        assert_eq!(
            state.disabled_patterns(None),
            [
                "re:vw_chrome_(status|task|execute)",
                "re:browser_browser_.+"
            ]
        );
        assert_eq!(
            state.request("s", ToolSet::YourChrome, Some("t1".into()), 0),
            Requested::TurningOn
        );
        // Pending is not on: the session still opens without the tools until the turn ends.
        assert_eq!(state.disabled_patterns(Some("s")).len(), 2);
        let pending = state.take_pending("s").unwrap();
        assert_eq!(pending.turn_id.as_deref(), Some("t1"));
        assert_eq!(
            state.disabled_patterns(Some("s")),
            ["re:browser_browser_.+"]
        );
        assert_eq!(state.disabled_patterns(Some("other")).len(), 2);
        assert_eq!(
            state.request("s", ToolSet::YourChrome, None, 0),
            Requested::AlreadyOn
        );
        assert!(
            state.take_pending("s").is_none(),
            "nothing waits after an already-on call"
        );
    }

    #[test]
    fn hand_off_moves_and_delete_forgets() {
        let state = OnDemand::default();
        state.allow_chrome("a");
        state.request("a", ToolSet::YourChrome, None, 0);
        state.take_pending("a");
        state.request("a", ToolSet::HeadlessBrowser, None, 0);
        state.hand_off("a", "b");
        assert!(state.chrome_allowed("b"));
        assert!(!state.chrome_allowed("a"));
        assert_eq!(state.disabled_patterns(Some("a")).len(), 2);
        assert_eq!(
            state.disabled_patterns(Some("b")),
            ["re:browser_browser_.+"]
        );
        assert!(state.has_pending("b"));
        state.forget("b");
        assert!(!state.chrome_allowed("b"));
        assert_eq!(state.disabled_patterns(Some("b")).len(), 2);
        assert!(!state.has_pending("b"));
    }

    #[test]
    fn the_core_patterns_join_the_callers_own_without_duplicates() {
        let mut config = serde_json::Map::new();
        config.insert(
            "disabledTools".into(),
            json!(["bash", "re:vw_chrome_(status|task|execute)"]),
        );
        add_patterns(
            &mut config,
            &[
                "re:vw_chrome_(status|task|execute)",
                "re:browser_browser_.+",
            ],
        );
        assert_eq!(
            config["disabledTools"],
            json!([
                "bash",
                "re:vw_chrome_(status|task|execute)",
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
        assert!(approve_without_dialog(&card("mcp_vw.use_your_chrome")));
        assert!(approve_without_dialog(&card("mcp_vw.use_headless_browser")));
        assert!(!approve_without_dialog(&card("mcp_vw.process_start")));
        assert!(!approve_without_dialog(&card(
            "mcp_chrome_control.chrome_page_commit"
        )));
        assert!(!approve_without_dialog(&card("use_your_chrome")));
        let question = json!({"detail": {"kind": "question", "effect": {"toolName": "mcp_vw.use_your_chrome"}}});
        assert!(!approve_without_dialog(&question));
    }

    #[test]
    fn the_words_the_agent_reads() {
        let on = ToolSet::YourChrome.turning_on();
        assert!(
            on.starts_with("Chrome Control's tools turn on for this chat when this reply ends"),
            "{on}"
        );
        assert!(on.contains("End this reply now"), "{on}");
        let line = ToolSet::YourChrome.continue_line();
        assert_eq!(
            line,
            "Chrome Control's tools are on in this chat now (group `mcp_vw`; find them with `search_tool_functions`). Continue the user's last request in their own Chrome. Every Chrome step follows the user's Chrome settings. A Chrome step that comes back \"Tool execution was skipped by Runtime policy\" was not done: the user refused it in Vibe Work; call `tools.mcp_vw.chrome_outcome({})` for the user's answer and tell the user."
        );
        assert!(
            ToolSet::YourChrome
                .not_there()
                .contains("Settings › Browser › Your Chrome")
        );
    }
}
