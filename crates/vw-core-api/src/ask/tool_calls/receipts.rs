//! Single-use decisions issued by the authenticated hook after permission is settled.
//! Binds owner, source/call, tool and input; the tool consumes the prepared action once, and only after the core saw
//! Vibe carry the receipt into a running call of the receipt's own chat (`announced`).
use super::announced::{self, Announcement, Origin};
use super::{Calls, PoisonError, Shared, Value, json, names};
use std::sync::Arc;
use std::time::Duration;

/// The refusal for a receipt no running Vibe call of its chat carries.
pub(crate) const NOT_STARTED_BY_VIBE: &str =
    "Not run: this tool call was not started by Vibe in this chat.";
/// How long the tool route waits for Vibe's report of a call; the tool server's request can overtake it.
const ANNOUNCE_WAIT: Duration = Duration::from_secs(3);
/// The wait between two reads of a session's history from Vibe.
const READ_AGAIN: Duration = Duration::from_millis(750);
/// How long a reported decision stays usable: Vibe's own limit for a session tool call. A call Vibe gave up on (its
/// tool server died, the request was dropped) cannot be completed later by whoever reads the decision from history.
const REPORTED_LIFETIME: Duration = Duration::from_secs(60);

pub(crate) type Execution =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value, String>> + Send>>;
pub(super) struct Receipt {
    key: String,
    owner: String,
    source: String,
    call: String,
    tool: String,
    args: Value,
    prepared_args: Value,
    execution: Option<Execution>,
    stop: u64,
    /// Vibe's report of this receipt in a running call of its own source session (the first report counts), and when
    /// it arrived (`REPORTED_LIFETIME`).
    announced: Option<(Announcement, std::time::Instant)>,
}
fn clean(args: &Value) -> Result<Value, String> {
    let mut args = args
        .as_object()
        .ok_or("Tool input must be an object.")?
        .clone();
    args.remove(names::RECEIPT_FIELD);
    let mut args = Value::Object(args);
    whole_numbers(&mut args);
    Ok(args)
}
/// One spelling per JSON number: a whole number written as a float (`1793404800000.0`, as a JavaScript runtime may
/// send `Date.now()`) becomes the integer the other copy of the same input has, so the comparison sees one value.
/// Only exact whole numbers within 2^53 change; any other number keeps its spelling.
fn whole_numbers(value: &mut Value) {
    match value {
        Value::Number(number) if number.is_f64() => {
            const EXACT: f64 = 9_007_199_254_740_992.0;
            if let Some(float) = number
                .as_f64()
                .filter(|f| f.fract() == 0.0 && f.abs() <= EXACT)
            {
                #[allow(
                    clippy::cast_possible_truncation,
                    reason = "a whole number within 2^53 is exact"
                )]
                let whole = float as i64;
                *value = json!(whole);
            }
        }
        Value::Array(items) => items.iter_mut().for_each(whole_numbers),
        Value::Object(fields) => fields.values_mut().for_each(whole_numbers),
        _ => {}
    }
}

/// Only the hook credential reaches this route; model arguments cannot supply the source or call.
pub(crate) async fn issue(shared: &Arc<Shared>, payload: &Value) -> Result<Value, String> {
    let field = |key: &str| {
        payload[key]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 256)
            .ok_or_else(|| format!("Missing or invalid {key}."))
    };
    let source = field("session_id")?;
    let call = field("tool_call_id")?;
    let owner = payload["parent_session_id"]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or(source);
    let tool = field("tool_name")?
        .strip_prefix("mcp_vw.")
        .ok_or("Not a Vibe Work tool.")?;
    let mut args = clean(&payload["tool_input"])?;
    let tools = shared
        .agent_tools
        .as_ref()
        .ok_or("Tools are unavailable.")?;
    if !tools.table.has_session(owner) {
        return Err("The originating chat is not registered.".into());
    }
    let stop = crate::stops::generation(shared, owner);
    let mut reservation = None;
    let mut replay = None;
    match super::browser_requests::admit(shared, owner, source, call, tool, &args)? {
        Some(super::browser_requests::Admission::New(guard)) => reservation = Some(guard),
        Some(super::browser_requests::Admission::Issued(key)) => {
            args[names::RECEIPT_FIELD] = json!(key);
            return Ok(args);
        }
        Some(super::browser_requests::Admission::Finished(answer)) => replay = Some(answer),
        None => {}
    }
    // Resolve a restart before opening its card and retain that exact launch in the one-use receipt.
    let restart = (tool == names::PROCESS_RESTART)
        .then(|| crate::agent_tools::restart_launch(shared, tools, owner, &args))
        .transpose()?;
    let mut prepared_args = args.clone();
    if let Some(launch) = &restart {
        prepared_args["command"] = json!(launch.command);
        prepared_args["cwd"] = json!(launch.cwd);
        prepared_args["port"] = json!(launch.port);
    }
    // The tool's own refusal (a vault Deny, a login-fill Cancel, a missing program) answers the agent as the
    // tool's result: inside run_typescript, Vibe shows a hook denial only as a generic skip, and the agent
    // retried the call. A No in a Vibe Work prompt asked here (the document card, a Chrome card, the vault window)
    // is such a refusal too: the hook answers with a decision whose tool call returns `vw_ask::DECLINED` and runs
    // nothing, and the turn goes on. A No that stopped the turn stays a denial with its own text.
    // A finished browser request replays its retained answer: no second card, no repeated input.
    let (prepared, shown) = if let Some(answer) = replay {
        let replayed = Box::pin(std::future::ready(super::browser_requests::replay(answer)));
        (Ok(Some(replayed as Execution)), false)
    } else {
        crate::approval_notice::observe(async {
            super::permission(shared, owner, source, call, tool, &prepared_args).await?;
            crate::agent_tools::prepare(shared, owner, source, call, tool, &mut prepared_args).await
        })
        .await
    };
    let execution = match prepared {
        Ok(execution) => execution,
        Err(refusal) if crate::stops::generation(shared, owner) != stop => {
            if let Some(guard) = reservation.take() {
                guard.finish(&Err(refusal.clone()));
            }
            return Err(refusal);
        }
        Err(refusal) => Some(Box::pin(std::future::ready(Err(refusal))) as Execution),
    };
    if shown {
        tools.one_ask.note_allowed(&[source, owner], call);
    }
    if let Some(launch) = restart {
        prepared_args["_vwRestart"] = json!(launch);
    }
    if crate::stops::generation(shared, owner) != stop {
        return Err(crate::stops::APPROVAL_ENDED.into());
    }
    let key = vw_agent_tools::credential::new_credential().map_err(|e| e.to_string())?;
    save(
        shared,
        Receipt {
            key: key.clone(),
            owner: owner.into(),
            source: source.into(),
            call: call.into(),
            tool: tool.into(),
            args: args.clone(),
            prepared_args,
            execution,
            stop,
            announced: None,
        },
    )?;
    if let Some(guard) = reservation {
        guard.issued(key.clone());
    }
    #[cfg(feature = "video")]
    if tool == "video_frames" {
        // Vibe's effect entry carries this receipt, not the hook's call id: the saved frames are found by it.
        tools.video_history.bind_receipt(source, call, &key);
    }
    args[names::RECEIPT_FIELD] = json!(key);
    Ok(args)
}
fn save(shared: &Shared, receipt: Receipt) -> Result<(), String> {
    let calls = super::calls(shared).ok_or(vw_vocabulary::texts::AGENT_TOOLS_UNAVAILABLE)?;
    let mut pending = calls
        .transports
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    pending.retain(|r| {
        crate::stops::generation(shared, &r.owner) == r.stop
            && r.announced
                .as_ref()
                .is_none_or(|(_, at)| at.elapsed() <= REPORTED_LIFETIME)
    });
    if pending.len() >= 1000 {
        return Err("Too many tool calls are in progress.".into());
    }
    pending.push_back(receipt);
    Ok(())
}

/// Records Vibe's report of a receipt in a newly added running call entry (the pump, for every notification on a
/// chat's channel, before the store applies it). Lobby notifications and entries for another tool are ignored.
pub(crate) fn observe(
    shared: &Shared,
    channel: Option<crate::channels::ChannelKey>,
    method: &str,
    params: &Value,
) {
    let (Some(channel), Some(entry)) = (channel, announced::notification(method, params)) else {
        return;
    };
    let origin = Origin::Channel(channel);
    if let Some(calls) = super::calls(shared) {
        mark(calls, entry.receipt, entry.tool, entry.session, origin);
    }
}

/// Sets the announcement of the live receipt `key` once, when the report names its own tool and source session (a
/// report for another session cannot block the real one), and wakes the tool routes waiting for it.
fn mark(calls: &Calls, key: &str, tool: &str, session: &str, origin: Origin) -> bool {
    let marked = calls
        .transports
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter_mut()
        .find(|r| r.key == key && r.tool == tool && r.source == session && r.announced.is_none())
        .map(|receipt| {
            receipt.announced = Some((
                Announcement {
                    session: session.to_owned(),
                    origin,
                },
                std::time::Instant::now(),
            ));
        })
        .is_some();
    if marked {
        calls.announced.notify_waiters();
    }
    marked
}

/// Debug-build test fixtures only: stands in for Vibe's report of `key` in its own source session, for direct tool
/// fixtures that call the hook and the tool route without a model. Never reachable from any route.
#[cfg(any(test, debug_assertions))]
pub(crate) fn announce_for_fixture(shared: &Shared, key: &str) -> bool {
    let Some(calls) = super::calls(shared) else {
        return false;
    };
    let found = calls
        .transports
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .find(|r| r.key == key)
        .map(|r| (r.source.clone(), r.tool.clone()));
    found.is_some_and(|(source, tool)| mark(calls, key, &tool, &source, Origin::Fixture))
}

/// Before the tool route consumes the receipt in `args`: waits a bounded time for Vibe's report of it, reading the
/// source session's history back from Vibe when no report is pushed (a subagent's calls are not pushed on its
/// parent's channel; a lost notification is read back the same way). `take` decides on what was seen.
pub(crate) async fn await_vibe_call(shared: &Arc<Shared>, caller: &str, args: &Value) {
    let Some(key) = args[names::RECEIPT_FIELD].as_str() else {
        return;
    };
    let Some(calls) = super::calls(shared) else {
        return;
    };
    let deadline = tokio::time::Instant::now() + ANNOUNCE_WAIT;
    let mut read = None;
    loop {
        let notified = calls.announced.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        let state = calls
            .transports
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .find(|r| r.key == key && r.owner == caller)
            .map(|r| (r.announced.is_some(), r.source.clone(), r.tool.clone()));
        let Some((false, source, tool)) = state else {
            // Unknown, used, another chat's or already reported: `take` answers.
            return;
        };
        // A child's history is read at once; a chat's own call is normally pushed before its tool request arrives.
        // Only the chat itself or a subagent the core's own record puts under it is ever read.
        let readable =
            || source == caller || crate::stop_tree::family(shared, caller).contains(&source);
        if (read.is_some() || source != caller) && readable() {
            let reply = tokio::time::timeout_at(
                deadline,
                shared.gate.call(
                    "session/read",
                    json!({"sessionId":source, "history":{"limit":50}}),
                    None,
                ),
            )
            .await;
            if reply.is_ok_and(|reply| {
                reply.is_ok_and(|reply| announced::in_history(&reply.result, &source, &tool, key))
            }) {
                mark(calls, key, &tool, &source, Origin::Read);
                return;
            }
        }
        let now = tokio::time::Instant::now();
        if now >= deadline {
            return;
        }
        let _ = tokio::time::timeout_at(deadline.min(now + READ_AGAIN), notified).await;
        read = Some(());
    }
}

fn index(
    pending: &std::collections::VecDeque<Receipt>,
    owner: &str,
    tool: &str,
    args: &Value,
) -> Result<usize, String> {
    let key = args[names::RECEIPT_FIELD]
        .as_str()
        .ok_or("This tool needs its pre-tool hook decision.")?;
    let index = pending
        .iter()
        .position(|r| r.key == key)
        .ok_or("Unknown or already used tool decision.")?;
    let receipt = &pending[index];
    if receipt.owner != owner || receipt.tool != tool || receipt.args != clean(args)? {
        return Err("Tool decision does not match this call.".into());
    }
    Ok(index)
}
pub(super) fn verify(
    calls: &Calls,
    owner: &str,
    source: &str,
    callback_id: &str,
    tool: &str,
    args: &Value,
) -> Result<(), String> {
    let pending = calls
        .transports
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let receipt = &pending[index(&pending, owner, tool, args)?];
    if receipt.source != source
        || !crate::one_ask::callback_matches_call(callback_id, &receipt.call)
    {
        return Err("This callback does not match its originating Vibe call.".into());
    }
    Ok(())
}
pub(crate) struct Prepared {
    pub(crate) execution: Option<Execution>,
    pub(crate) approved: bool,
}

pub(crate) fn take(
    shared: &Arc<Shared>,
    owner: &str,
    tool: &str,
    args: &mut Value,
) -> Result<Prepared, String> {
    let calls = super::calls(shared).ok_or(vw_vocabulary::texts::AGENT_TOOLS_UNAVAILABLE)?;
    let mut pending = calls
        .transports
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let index = index(&pending, owner, tool, args)?;
    let receipt = pending.remove(index).ok_or("Tool decision already used.")?;
    drop(pending);
    // The final dispatch binding: only a call Vibe itself runs in this chat (or its own subagent) may use it. A refused
    // receipt is spent, so a forged one cannot be presented again.
    let fresh = receipt
        .announced
        .as_ref()
        .is_some_and(|(_, at)| at.elapsed() <= REPORTED_LIFETIME);
    if !fresh
        || !announced::started_by_vibe(
            receipt.announced.as_ref().map(|(seen, _)| seen),
            &receipt.owner,
            &receipt.source,
            shared.channels.key_of(&receipt.owner),
            || crate::stop_tree::family(shared, &receipt.owner).contains(&receipt.source),
        )
    {
        tracing::warn!(
            tool = %receipt.tool,
            announced = receipt.announced.is_some(),
            "tool decision refused: no running Vibe call of its chat carries it"
        );
        return Err(NOT_STARTED_BY_VIBE.into());
    }
    if crate::stops::generation(shared, owner) != receipt.stop {
        return Err(crate::stops::APPROVAL_ENDED.into());
    }
    *args = receipt.prepared_args.clone();
    let approved = crate::one_ask::of(shared)
        .is_some_and(|ledger| ledger.user_allowed(&receipt.source, &receipt.call));
    if super::browser_requests::tracked(tool) {
        return browser_execution(shared, receipt).map(|execution| Prepared {
            execution: Some(execution),
            approved,
        });
    }
    Ok(Prepared {
        execution: receipt.execution,
        approved,
    })
}

fn browser_execution(shared: &Arc<Shared>, receipt: Receipt) -> Result<Execution, String> {
    let tools = shared
        .agent_tools
        .as_ref()
        .ok_or(vw_vocabulary::texts::AGENT_TOOLS_UNAVAILABLE)?
        .clone();
    let guard = tools.tool_asks.browser_requests.running(
        (
            receipt.owner.clone(),
            receipt.source.clone(),
            receipt.call.clone(),
        ),
        &receipt.key,
    )?;
    let shared = Arc::downgrade(shared);
    Ok(Box::pin(async move {
        let shared = shared.upgrade().ok_or("Vibe Work closed.")?;
        // Cached receipts never preserve a mode that has since lost site-secret access.
        super::browser_requests::check_full(&shared, &receipt.owner, &receipt.tool, &receipt.args)?;
        let answer = match receipt.execution {
            Some(execution) => execution.await,
            None => {
                crate::agent_tools::call_prepared(
                    &shared,
                    &tools,
                    &receipt.owner,
                    &receipt.tool,
                    receipt.prepared_args,
                )
                .await
            }
        };
        if let Some(guard) = guard {
            guard.finish(&answer);
        }
        answer
    }))
}

#[cfg(test)]
#[path = "receipts_tests.rs"]
mod tests;

#[cfg(test)]
mod decline_tests {
    use super::*;
    #[tokio::test]
    async fn denying_a_tool_card_returns_the_decline_as_the_tool_result() {
        let f = crate::feature_test_core::Fixture::new().await;
        f.load("s", &f.root.join("project"), &[], false);
        let shared = &f.core.shared;
        shared
            .agent_tools
            .as_ref()
            .unwrap()
            .table
            .for_session("s")
            .unwrap();
        let owner = shared.clone();
        let task = tokio::spawn(async move {
            issue(
                &owner,
                &json!({"session_id":"s", "tool_call_id":"read-1",
            "tool_name":"mcp_vw.search", "tool_input":{"query":"needle"}}),
            )
            .await
        });
        let ask = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let Some(ask) = f.core.asks().into_iter().next() {
                    break ask;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        super::super::answer_by(
            shared,
            crate::Origin::Ui,
            "s",
            ask.id.strip_prefix("callback:").unwrap(),
            &json!({"decision":{"type":"deny"}}),
            vw_ask::AnsweredBy::Person,
        )
        .unwrap();
        let decision = task.await.unwrap();
        assert!(
            decision.is_ok(),
            "the hook must retain a declined tool result: {decision:?}"
        );
        let mut args = decision.unwrap();
        assert!(announce_for_fixture(
            shared,
            args[names::RECEIPT_FIELD].as_str().unwrap()
        ));
        let result = take(shared, "s", "search", &mut args)
            .unwrap()
            .execution
            .expect("held refusal")
            .await;
        assert!(result.unwrap_err().contains("the user declined"));
        assert!(take(shared, "s", "search", &mut args).is_err());
        f.finish().await;
    }

    /// Whoever holds the hook credential can mint a receipt naming any registered chat.
    /// PREDICT (before the fix): `take` accepts the minted receipt because owner, tool and input match, so the tool
    /// would run for a call Vibe never made; after the fix it answers `NOT_STARTED_BY_VIBE` and spends the receipt.
    #[tokio::test]
    async fn a_receipt_no_vibe_call_carries_is_refused_and_spent() {
        let f = crate::feature_test_core::Fixture::new().await;
        f.load("s", &f.root.join("project"), &[], false);
        let shared = &f.core.shared;
        shared
            .agent_tools
            .as_ref()
            .unwrap()
            .table
            .for_session("s")
            .unwrap();
        shared
            .access
            .observe_runtime("s", &json!({"activeAgent":{"name":"auto-approve"}}));
        let mut args = issue(
            shared,
            &json!({"session_id":"s", "tool_call_id":"forged-1",
            "tool_name":"mcp_vw.search", "tool_input":{"query":"needle"}}),
        )
        .await
        .unwrap();
        let mut again = args.clone();
        let refused = take(shared, "s", "search", &mut args).err();
        assert_eq!(refused.as_deref(), Some(NOT_STARTED_BY_VIBE));
        assert!(
            take(shared, "s", "search", &mut again)
                .err()
                .is_some_and(|e| e.contains("already used")),
            "a refused receipt is spent"
        );
        f.finish().await;
    }

    /// PREDICT: only a newly added running entry of the receipt's own tool, pushed on a channel, marks it; a lobby
    /// notification or another tool's entry does not. A marked receipt still needs the owner's live channel.
    #[tokio::test]
    async fn a_pushed_report_marks_only_its_own_tool_and_needs_the_owners_channel() {
        let f = crate::feature_test_core::Fixture::new().await;
        f.load("s", &f.root.join("project"), &[], false);
        let shared = &f.core.shared;
        shared
            .agent_tools
            .as_ref()
            .unwrap()
            .table
            .for_session("s")
            .unwrap();
        shared
            .access
            .observe_runtime("s", &json!({"activeAgent":{"name":"auto-approve"}}));
        let mut args = issue(
            shared,
            &json!({"session_id":"s", "tool_call_id":"call-1",
            "tool_name":"mcp_vw.search", "tool_input":{"query":"needle"}}),
        )
        .await
        .unwrap();
        let key = args[names::RECEIPT_FIELD].as_str().unwrap().to_owned();
        let added = |tool: &str| {
            json!({"sessionId":"s","entry":{"type":"effect","id":"e","sessionId":"s",
            "state":{"status":"running"},"detail":{"toolName":tool,"input":{"query":"needle","_vwReceipt":key}}}})
        };
        let announced = |shared: &Shared| {
            super::super::calls(shared)
                .unwrap()
                .transports
                .lock()
                .unwrap()
                .iter()
                .find(|r| r.key == key)
                .and_then(|r| r.announced.as_ref().map(|(seen, _)| seen.clone()))
        };
        observe(shared, None, "history/entryAdded", &added("mcp_vw.search"));
        observe(
            shared,
            Some((1, 1)),
            "history/entryAdded",
            &added("mcp_vw.secret_run"),
        );
        assert_eq!(announced(shared), None);
        observe(
            shared,
            Some((1, 1)),
            "history/entryAdded",
            &added("mcp_vw.search"),
        );
        assert_eq!(
            announced(shared),
            Some(Announcement {
                session: "s".into(),
                origin: Origin::Channel((1, 1))
            })
        );
        // This fixture's chat has no live channel, so no channel can be its own.
        assert_eq!(
            take(shared, "s", "search", &mut args).err().as_deref(),
            Some(NOT_STARTED_BY_VIBE)
        );
        f.finish().await;
    }

    /// PREDICT: a report naming another session neither marks the receipt nor blocks Vibe's own later report; a
    /// report older than Vibe's tool-call limit no longer lets the receipt run.
    #[tokio::test]
    async fn only_a_fresh_report_of_the_receipts_own_session_counts() {
        let f = crate::feature_test_core::Fixture::new().await;
        f.load("s", &f.root.join("project"), &[], false);
        let shared = &f.core.shared;
        shared
            .agent_tools
            .as_ref()
            .unwrap()
            .table
            .for_session("s")
            .unwrap();
        shared
            .access
            .observe_runtime("s", &json!({"activeAgent":{"name":"auto-approve"}}));
        let payload = |call: &str| {
            json!({"session_id":"s", "tool_call_id":call,
            "tool_name":"mcp_vw.search", "tool_input":{"query":"needle"}})
        };
        let calls = super::super::calls(shared).unwrap();
        let mut args = issue(shared, &payload("call-1")).await.unwrap();
        let key = args[names::RECEIPT_FIELD].as_str().unwrap().to_owned();
        assert!(!mark(calls, &key, "search", "other", Origin::Fixture));
        assert!(mark(calls, &key, "search", "s", Origin::Fixture));
        assert!(take(shared, "s", "search", &mut args).is_ok());

        let mut stale = issue(shared, &payload("call-2")).await.unwrap();
        let key = stale[names::RECEIPT_FIELD].as_str().unwrap().to_owned();
        for receipt in calls.transports.lock().unwrap().iter_mut() {
            if receipt.key == key {
                let long_ago = std::time::Instant::now()
                    .checked_sub(REPORTED_LIFETIME + Duration::from_secs(1))
                    .unwrap();
                receipt.announced = Some((
                    Announcement {
                        session: "s".into(),
                        origin: Origin::Fixture,
                    },
                    long_ago,
                ));
            }
        }
        assert_eq!(
            take(shared, "s", "search", &mut stale).err().as_deref(),
            Some(NOT_STARTED_BY_VIBE)
        );
        f.finish().await;
    }
}
