//! Browser request identity survives one-use transport receipts and lost connections.
//! The hook reserves owner/source/call before consent; bounded completed results never replay native input.
use super::{Shared, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex, PoisonError},
};

type Answer = Result<Value, String>;
pub(super) type Key = (String, String, String);
const RESULT_BYTES: usize = 262_144;
const RESULT_COUNT: usize = 64;
const REQUEST_COUNT: usize = 10_000;
const UNRETAINED: &str = "This browser request already ran, but its result is no longer retained. No input was repeated. Inspect the current page before choosing another action.";
#[derive(Default)]
pub(crate) struct Ledger(Mutex<Records>);
#[derive(Default)]
struct Records {
    entries: BTreeMap<Key, Entry>,
    results: VecDeque<Key>,
}
struct Entry {
    digest: [u8; 32],
    stop: u64,
    state: State,
}
enum State {
    Preparing,
    Issued(String),
    Running,
    Finished(Option<Answer>),
    Interrupted,
}
pub(super) enum Admission {
    New(Reservation),
    Issued(String),
    Finished(Answer),
}
pub(super) struct Reservation {
    ledger: Arc<Ledger>,
    key: Key,
    active: bool,
}
pub(super) fn tracked(tool: &str) -> bool {
    matches!(tool, "browser_act" | "chrome_execute")
}
pub(super) fn check_full(
    shared: &Shared,
    owner: &str,
    tool: &str,
    args: &Value,
) -> Result<(), String> {
    let mode = shared.access.remembered(owner);
    if tool == "browser_act" {
        crate::chrome_control::builtin::check_full(mode.as_deref(), args)
    } else {
        crate::chrome_control::check::check_full(shared, owner, tool, args)
    }
}
pub(super) fn admit(
    shared: &Shared,
    owner: &str,
    source: &str,
    call: &str,
    tool: &str,
    args: &Value,
) -> Result<Option<Admission>, String> {
    if !tracked(tool) {
        return Ok(None);
    }
    let tools = shared
        .agent_tools
        .as_ref()
        .ok_or(vw_vocabulary::texts::AGENT_TOOLS_UNAVAILABLE)?;
    let admission = tools.tool_asks.browser_requests.begin(
        (owner.into(), source.into(), call.into()),
        tool,
        args,
        crate::stops::generation(shared, owner),
    )?;
    if !matches!(&admission, Admission::New(_)) {
        check_full(shared, owner, tool, args)?;
    }
    Ok(Some(admission))
}
pub(super) fn replay(answer: Answer) -> Answer {
    const NOTE: &str = "This is the original request's result; no action was repeated. The current page may have changed since that result.";
    let mut value = answer.map_err(|reason| format!("{NOTE} {reason}"))?;
    if let Some(mcp) = value.get_mut("vwMcpResult") {
        if let Some(body) = mcp["structuredContent"].as_object_mut() {
            body.insert("requestReplay".into(), serde_json::json!(true));
        }
        if let Some(content) = mcp["content"].as_array_mut() {
            content.insert(0, serde_json::json!({"type":"text","text":NOTE}));
        }
    } else if let Some(body) = value.as_object_mut() {
        body.insert("requestReplay".into(), serde_json::json!(true));
        body.insert("requestReplayNote".into(), serde_json::json!(NOTE));
    }
    Ok(value)
}
impl Ledger {
    pub(super) fn begin(
        self: &Arc<Self>,
        key: Key,
        tool: &str,
        args: &Value,
        stop: u64,
    ) -> Result<Admission, String> {
        let encoded = serde_json::to_vec(&(tool, args))
            .map_err(|_| "Browser request could not be encoded. Supply valid action fields.")?;
        let digest: [u8; 32] = Sha256::digest(encoded).into();
        let mut records = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(entry) = records.entries.get(&key) {
            if entry.digest != digest {
                return Err("This browser call id was already used with different input. Keep the original request unchanged; a deliberate new action needs a new call id.".into());
            }
            if entry.stop != stop {
                return Err(crate::stops::APPROVAL_ENDED.into());
            }
            return match &entry.state {
                State::Issued(key) => Ok(Admission::Issued(key.clone())),
                State::Finished(Some(answer)) => Ok(Admission::Finished(answer.clone())),
                State::Finished(None) => Err(UNRETAINED.into()),
                State::Preparing | State::Running => Err("This browser request is already in progress. Wait for its original result; no second action was dispatched.".into()),
                State::Interrupted => Err("The original browser request ended without a confirmed result. Its outcome may be unknown; no input was repeated. Inspect the current page before choosing another action.".into()),
            };
        }
        if records.entries.len() >= REQUEST_COUNT {
            return Err("The browser request ledger is full. Close completed chats before starting more browser work.".into());
        }
        records.entries.insert(
            key.clone(),
            Entry {
                digest,
                stop,
                state: State::Preparing,
            },
        );
        Ok(Admission::New(Reservation {
            ledger: self.clone(),
            key,
            active: true,
        }))
    }
    pub(super) fn running(
        self: &Arc<Self>,
        key: Key,
        receipt: &str,
    ) -> Result<Option<Reservation>, String> {
        let mut records = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(entry) = records.entries.get_mut(&key) else {
            return Ok(None);
        };
        match &entry.state {
            State::Issued(issued) if issued == receipt => {
                entry.state = State::Running;
                Ok(Some(Reservation{ledger:self.clone(),key,active:true}))
            }
            State::Finished(_) => Ok(None),
            _ => Err("This browser request has no unused execution decision. Use its original result or inspect the page if the connection ended.".into()),
        }
    }
    pub(crate) fn forget(&self, owner: &str) {
        let mut records = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        records.entries.retain(|(chat, _, _), _| chat != owner);
        records.results.retain(|(chat, _, _)| chat != owner);
    }
}
impl Reservation {
    pub(super) fn issued(mut self, receipt: String) {
        if let Some(entry) = self
            .ledger
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entries
            .get_mut(&self.key)
        {
            entry.state = State::Issued(receipt);
        }
        self.active = false;
    }
    pub(super) fn finish(mut self, answer: &Answer) {
        let mut records = self.ledger.0.lock().unwrap_or_else(PoisonError::into_inner);
        let retain = serde_json::to_vec(answer).is_ok_and(|v| v.len() <= RESULT_BYTES);
        if let Some(entry) = records.entries.get_mut(&self.key) {
            entry.state = State::Finished(retain.then(|| answer.clone()));
        }
        if retain {
            records.results.push_back(self.key.clone());
            while records.results.len() > RESULT_COUNT {
                if let Some(key) = records.results.pop_front()
                    && let Some(entry) = records.entries.get_mut(&key)
                {
                    entry.state = State::Finished(None);
                }
            }
        }
        self.active = false;
    }
}
impl Drop for Reservation {
    fn drop(&mut self) {
        if self.active
            && let Some(entry) = self
                .ledger
                .0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .entries
                .get_mut(&self.key)
        {
            entry.state = State::Interrupted;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn key(call: &str) -> Key {
        ("owner".into(), "source".into(), call.into())
    }
    fn reserve(ledger: &Arc<Ledger>, call: &str) -> Reservation {
        match ledger
            .begin(
                key(call),
                "browser_act",
                &json!({"action":"window.open"}),
                0,
            )
            .unwrap()
        {
            Admission::New(guard) => guard,
            _ => panic!("new request"),
        }
    }
    #[test]
    fn canceled_pending_requests_never_become_fresh_input_and_owner_keys_are_distinct() {
        let ledger = Arc::new(Ledger::default());
        let guard = reserve(&ledger, "one");
        assert!(
            ledger
                .begin(
                    key("one"),
                    "browser_act",
                    &json!({"action":"window.open"}),
                    0
                )
                .is_err()
        );
        drop(guard);
        assert!(
            ledger
                .begin(
                    key("one"),
                    "browser_act",
                    &json!({"action":"window.open"}),
                    0
                )
                .is_err()
        );
        assert!(matches!(
            ledger
                .begin(
                    ("another".into(), "source".into(), "one".into()),
                    "browser_act",
                    &json!({"action":"window.open"}),
                    0
                )
                .unwrap(),
            Admission::New(_)
        ));
    }
    #[test]
    fn result_eviction_keeps_the_no_replay_record_and_stop_revokes_cached_answers() {
        let ledger = Arc::new(Ledger::default());
        for n in 0..=RESULT_COUNT {
            reserve(&ledger, &n.to_string()).finish(&Ok(json!({"actual":n})));
        }
        assert!(
            ledger
                .begin(key("0"), "browser_act", &json!({"action":"window.open"}), 0)
                .is_err()
        );
        let call = RESULT_COUNT.to_string();
        assert!(matches!(
            ledger
                .begin(
                    key(&call),
                    "browser_act",
                    &json!({"action":"window.open"}),
                    0
                )
                .unwrap(),
            Admission::Finished(_)
        ));
        assert!(
            ledger
                .begin(
                    key(&call),
                    "browser_act",
                    &json!({"action":"window.open"}),
                    1
                )
                .is_err()
        );
        reserve(&ledger, "large").finish(&Ok(json!({"value":"x".repeat(RESULT_BYTES)})));
        assert!(
            ledger
                .begin(
                    key("large"),
                    "browser_act",
                    &json!({"action":"window.open"}),
                    0
                )
                .is_err()
        );
        ledger.forget("owner");
        assert!(ledger.0.lock().unwrap().entries.is_empty());
    }
    #[tokio::test]
    async fn cached_site_secrets_use_the_current_shared_full_access_gate() {
        let core = crate::offline_core::start("browser-cache-mode").await;
        for mode in ["auto-approve", "accept-edits", "ask"] {
            core.shared.access.observe_request(
                "session/agent/update",
                "s",
                &json!({"agentName":mode}),
            );
            for (tool, args) in [
                ("browser_act", json!({"action":"clipboard.read"})),
                (
                    "browser_act",
                    json!({"action":"clipboard.write_rich","mime":"text/html","text":"<p>Note</p>"}),
                ),
                (
                    "browser_act",
                    json!({"steps":[{"action":"window.list"},{"action":"storage.read"}]}),
                ),
                (
                    "chrome_execute",
                    json!({"action":"clipboard.read","args":{}}),
                ),
                (
                    "chrome_execute",
                    json!({"action":"clipboard.write_rich","args":{"mime":"text/html","text":"<p>Note</p>"}}),
                ),
            ] {
                assert_eq!(
                    check_full(&core.shared, "s", tool, &args).is_ok(),
                    mode == "auto-approve",
                    "{mode}: {tool}"
                );
            }
        }
        core.shutdown().await;
    }
}
