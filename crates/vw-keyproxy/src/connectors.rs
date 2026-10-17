//! The connector rule at the model proxy: a Mistral connector call that would WRITE to a
//! code forge never leaves the machine; the proxy answers it itself with the Push-button sentence
//! (`vw_agent_tools::forge::CONNECTOR_REASON`). Connector reads and every other connector pass unchanged.
//!
//! Why here: Vibe sends every connector call to `<api base>/v1/connectors-gateway/<connector id>/mcp` (unified
//! harness `_connector_gateway.py:129`, legacy `connector_registry.py:390`), and its api base is this proxy, which
//! alone holds the key. So the rule holds in every access mode, also with "approve for the session" and in
//! auto-approve, and it does not depend on Vibe running our hook.
//!
//! The request is MCP over streamable HTTP: a JSON-RPC message per POST (`initialize`, `notifications/initialized`,
//! `tools/call`, `tools/list`). Only `tools/call` is judged, on its `params.name` (the remote tool, `push_files`) and
//! the connector's alias, which the proxy looks up by the id in the path in [`Aliases`]: learned from the catalog
//! answers (`GET /v1/connectors/bootstrap`, `{connectors: [{id, name},...]}`) the proxy itself relays from Mistral,
//! kept in memory only (never from a file the agent could write). Vibe Work removes Vibe's on-disk
//! catalog cache at launch (`vw_host::launch`), so Vibe loads the catalog through the proxy at least once per run. An id
//! the proxy has not seen is judged as a forge (fails closed: only read-shaped tools pass), and so is an id whose
//! entries disagree.
//!
//! The refusal is a JSON-RPC **success** result whose text is the sentence, so the model reads the reason as the
//! connector tool's answer (Vibe Work answers every refusal this way). The text starts with "Not run:", so it is
//! never read as a done push.
//!
//! Depends on: vw-agent-tools (`forge`), `serde_json`. Called by: `server`.
//! Invariants:
//! - a refused call is never forwarded; a body over [`MAX_BODY`] or not JSON is refused (never forwarded unread);
//! - only the gateway body's `method`, `id` and `params.name` are read; catalog tool descriptions are filled before
//!   Vibe sees them so its harness never invents a placeholder for a missing upstream description;
//!   nothing is logged but the tool name and the decision.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::{Value, json};
use vw_agent_tools::forge;

/// The proxy path template of the connector gateway (same string as the allowlist entry in `routes.rs`).
pub const GATEWAY_TEMPLATE: &str = "/v1/connectors-gateway/{}/mcp";

/// Longest connector request body the proxy reads (a `push_files` with many files is the large case; refusing a
/// bigger body fails closed).
pub const MAX_BODY: usize = 32 * 1024 * 1024;

/// The proxy path of the connector catalog (same string as the allowlist entry in `routes.rs`).
pub const BOOTSTRAP_TEMPLATE: &str = "/v1/connectors/bootstrap";

/// Longest catalog answer the proxy reads to learn aliases (a bigger one is passed on unread: its ids stay unknown,
/// so they are judged as forges).
pub const MAX_CATALOG: usize = 16 * 1024 * 1024;

/// Fill missing connector descriptions before Vibe's harness turns them into `Connector tool <name>.`.
/// The three web tools have specific descriptions; other tools get a neutral label based on their names. A real
/// upstream description is left alone. `None` means the catalog was invalid or needed no change.
#[must_use]
pub fn describe_catalog(body: &[u8]) -> Option<Vec<u8>> {
    let mut value: Value = serde_json::from_slice(body).ok()?;
    let connectors = value.get_mut("connectors")?.as_array_mut()?;
    let mut changed = false;
    for connector in connectors {
        let alias = connector
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let Some(tools) = connector.get_mut("tools").and_then(Value::as_array_mut) else {
            continue;
        };
        for tool in tools {
            let Some(name) = tool.get("name").and_then(Value::as_str).map(str::to_owned) else {
                continue;
            };
            let description = tool.get("description").and_then(Value::as_str);
            if description
                .is_some_and(|d| !d.trim().is_empty() && !d.trim().starts_with("Connector tool "))
            {
                continue;
            }
            let description = match (alias.as_str(), name.as_str()) {
                ("web_search", "news_search") => {
                    "Search news articles with optional date and language filters.".to_owned()
                }
                ("web_search", "open_url") => {
                    "Open a web page or AFP/AP news article ID and return its content.".to_owned()
                }
                ("web_search", "web_search") => {
                    "Search the web for pages and return links, snippets, and publication dates."
                        .to_owned()
                }
                _ => {
                    let words = name.replace('_', " ");
                    let mut chars = words.chars();
                    let action = match chars.next() {
                        Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
                        None => continue,
                    };
                    if alias.is_empty() {
                        format!("{action} with this connector.")
                    } else {
                        format!(
                            "{action} through the {} connector.",
                            alias.replace('_', " ")
                        )
                    }
                }
            };
            let Some(object) = tool.as_object_mut() else {
                continue;
            };
            object.insert("description".to_owned(), Value::String(description));
            changed = true;
        }
    }
    if changed {
        serde_json::to_vec(&value).ok()
    } else {
        None
    }
}

/// Connector id -> the names Mistral's catalog gave it, learned from relayed catalog answers (memory only, shared by
/// every request of one proxy).
#[derive(Clone, Default)]
pub struct Aliases(Arc<Mutex<HashMap<String, Vec<String>>>>);

impl Aliases {
    /// Records every `{id, name}` of a catalog answer body (`{connectors: [...]}`); anything else is ignored.
    pub fn learn(&self, body: &[u8]) {
        let Ok(value) = serde_json::from_slice::<Value>(body) else {
            return;
        };
        let Some(list) = value.get("connectors").and_then(Value::as_array) else {
            return;
        };
        let mut map = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        for connector in list {
            let id = connector.get("id").and_then(Value::as_str).map(str::trim);
            let name = connector.get("name").and_then(Value::as_str);
            if let (Some(id), Some(name)) = (id.filter(|i| !i.is_empty()), name) {
                let names = map.entry(id.to_owned()).or_default();
                if !names.iter().any(|n| n == name) {
                    names.push(name.to_owned());
                }
            }
        }
    }

    /// The names learned for `connector_id` (empty when never seen).
    #[must_use]
    pub fn of(&self, connector_id: &str) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(connector_id)
            .cloned()
            .unwrap_or_default()
    }
}

/// What to do with one gateway request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Send it on unchanged.
    Forward,
    /// Answer it here with this JSON body (status 200): the refusal as the tool's result.
    Answer(Value),
    /// Refuse it with this reason (status 400): not JSON, or a batch that holds a refused call.
    Refuse(&'static str),
}

/// True when a `tools/call` of `tool` on a connector with `aliases` is refused: a forge alias (or no alias known, or
/// names that disagree: judged as a forge) and a tool that does not only read.
#[must_use]
pub fn refused(aliases: &[String], tool: &str) -> bool {
    let forge = aliases.len() != 1 || aliases.iter().any(|a| forge::is_forge(a));
    forge && !forge::is_read_only(tool)
}

fn refusal(id: &Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "content": [{"type": "text", "text": forge::CONNECTOR_REASON}],
            "isError": false,
        },
    })
}

/// Judges one gateway request body (`connector_id` from the path; `aliases` = what the proxy learned).
#[must_use]
pub fn judge(body: &[u8], connector_id: &str, aliases: &Aliases) -> Verdict {
    let Ok(message) = serde_json::from_slice::<Value>(body) else {
        return Verdict::Refuse(
            "Vibe Work's model proxy refused a connector request that is not JSON.",
        );
    };
    let calls: Vec<&Value> = match &message {
        Value::Array(items) => items.iter().collect(),
        single => vec![single],
    };
    let mut refused_ids: Vec<&Value> = Vec::new();
    let mut known: Option<Vec<String>> = None;
    for call in &calls {
        if call.get("method").and_then(Value::as_str) != Some("tools/call") {
            continue;
        }
        let tool = call
            .pointer("/params/name")
            .and_then(Value::as_str)
            .unwrap_or("");
        let names = known.get_or_insert_with(|| aliases.of(connector_id));
        if refused(names, tool) {
            tracing::info!(
                tool,
                "model proxy: connector write refused (the user's Push / Create PR buttons)"
            );
            refused_ids.push(call.get("id").unwrap_or(&Value::Null));
        }
    }
    match (refused_ids.as_slice(), &message) {
        ([], _) => Verdict::Forward,
        ([id], Value::Object(_)) => Verdict::Answer(refusal(id)),
        _ => Verdict::Refuse(
            "Vibe Work's model proxy refused a batch of connector calls that changes a repository: that is the \
             user's step (the Push and Create PR buttons).",
        ),
    }
}
