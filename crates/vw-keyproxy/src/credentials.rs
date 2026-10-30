//! The one connector-credential write the proxy forwards: making the user's own default connection to a connector
//! that needs no sign-in, with no secret in it.
//!
//! Vibe's `connector_catalog/connect` (Vibe 3.0.3 `app_server/connector_catalog.py` `_connect_without_credentials`)
//! does what Mistral's dashboard Connect button does: `PATCH /v1/connectors/<id>/user/credentials` with
//! `{"name": "default", "is_default": true, "credentials": {"headers": {}}}` (mistralai `connectors.py`
//! `update_credentials_async`, `consumer_scope="user"`). The allowlist (`routes.rs`) has only the user-scope path;
//! the organization and workspace scopes, reads and deletes of credentials stay refused. Before the key is added,
//! this module requires all three of:
//! - the body is exactly Vibe's (so no header, token or OAuth value, no other name or title, can be stored);
//! - the connector id is one the latest catalog answer the proxy relayed marks `no_auth_connect` (a connector with a
//!   sign-in or a setup, or an id never seen, is refused);
//! - that answer lists it as not connected, so a connection the person already has is never replaced; one forwarded
//!   connect spends the id until a newer catalog answer lists it as not connected again.
//!
//! The catalog facts come only from answers the proxy itself relayed from Mistral (memory only, never a file).
//! Depends on: `serde_json`. Called by: `connector_bodies` (the credential body and the catalog answer).
//! Invariant: anything else is refused 400 and never forwarded; nothing of the body is logged.

use std::collections::HashSet;
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::{Value, json};

/// The proxy path template of the user-scope credential write (same string as the allowlist entry in `routes.rs`).
pub const CREDENTIALS_TEMPLATE: &str = "/v1/connectors/{}/user/credentials";

/// Longest credential body the proxy reads (Vibe's is under 100 bytes).
pub const MAX_CREDENTIALS_BODY: usize = 4 * 1024;

/// The refusal sentence for any other credential call.
pub const REFUSED: &str = "Vibe Work's model proxy only connects a connector that needs no sign-in and is not connected yet, without any secret.";

/// True when `body` is the JSON object Vibe 3.0.3 sends for a no-sign-in connect, and nothing else:
/// `{"name": "default", "is_default": true, "credentials": {"headers": {}}}`.
#[must_use]
pub fn no_secret_connection(body: &[u8]) -> bool {
    serde_json::from_slice::<Value>(body).is_ok_and(|value| {
        value == json!({"name": "default", "is_default": true, "credentials": {"headers": {}}})
    })
}

/// The connector ids the latest relayed catalog answer marks as needing no sign-in and not connected yet.
#[derive(Clone, Debug, Default)]
pub struct Connectable(Arc<Mutex<HashSet<String>>>);

impl Connectable {
    /// Replaces the set with what a catalog answer body (`{connectors: [{id, status: {is_ready}, auth_action:
    /// {type}}]}`) says; a body that is not a catalog changes nothing.
    pub fn learn(&self, body: &[u8]) {
        let Ok(value) = serde_json::from_slice::<Value>(body) else {
            return;
        };
        let Some(list) = value.get("connectors").and_then(Value::as_array) else {
            return;
        };
        let ids = list
            .iter()
            .filter(|c| {
                c.pointer("/auth_action/type").and_then(Value::as_str) == Some("no_auth_connect")
            })
            .filter(|c| c.pointer("/status/is_ready").and_then(Value::as_bool) == Some(false))
            .filter_map(|c| c.get("id").and_then(Value::as_str))
            .map(|id| id.trim().to_owned())
            .filter(|id| !id.is_empty())
            .collect();
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = ids;
    }

    /// Takes `id` out of the set: true when it was there (the one connect it allows).
    #[must_use]
    pub fn take(&self, id: &str) -> bool {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(id)
    }
}
