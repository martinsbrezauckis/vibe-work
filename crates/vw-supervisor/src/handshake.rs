//! The `initialize` handshake: our client info and capabilities, then the `initialized`
//! notification.
//!
//! Depends on: vw-rpc, vw-protocol (`InitializeParams`, `InitializeResponse`). Called by:
//! `supervisor` after every spawn.
//! Invariant: a connection is handed to callers only after `initialize` answered and
//! `initialized` was sent (the server answers `not_initialized` otherwise, `protocol.py:2353`).

use std::time::Duration;

use serde_json::json;
use vw_protocol::{
    AgentEntrypoint, CallbackKind, ClientCapabilities, ClientInfo, InitializeParams,
    InitializeResponse, Maybe,
};
use vw_rpc::{Connection, RequestOptions, RpcError};

/// Default client identity: name `vibe-work`, this crate's version, entrypoint `desktop`, and
/// the callback kinds used by the client (`approval`, `user_input`).
#[must_use]
pub fn default_initialize_params() -> InitializeParams {
    InitializeParams {
        client_info: ClientInfo {
            name: "vibe-work".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            title: Maybe::Value("Vibe Work".to_owned()),
            entrypoint: Some(AgentEntrypoint::Desktop),
            terminal_emulator: None,
        },
        capabilities: Some(ClientCapabilities {
            callback_kinds: Some(vec![CallbackKind::Approval, CallbackKind::UserInput]),
            client_tools: None,
            disabled_notifications: None,
        }),
    }
}

/// Runs `initialize` (bounded by `timeout`) and sends `initialized`.
///
/// # Errors
/// Any [`RpcError`] from the request (a dead server, a timeout, an error answer, a result that
/// does not match the generated `InitializeResponse`).
pub async fn handshake(
    conn: &Connection,
    params: &InitializeParams,
    timeout: Duration,
) -> Result<InitializeResponse, RpcError> {
    let encoded = serde_json::to_value(params).map_err(|source| RpcError::Encode {
        method: "initialize".to_owned(),
        source,
    })?;
    let result = conn
        .request_with("initialize", encoded, RequestOptions::timeout(timeout))
        .await?;
    let response: InitializeResponse =
        serde_json::from_value(result).map_err(|source| RpcError::Decode {
            method: "initialize".to_owned(),
            source,
        })?;
    conn.notify("initialized", json!({})).await?;
    Ok(response)
}
