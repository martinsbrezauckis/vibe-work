//! The command table: which JSON-RPC method (and params) each `Command` becomes. Pure data, no
//! I/O, so the mapping can be read in one place and tested variant by variant.
//!
//! Depends on: `command`. Called by: `dispatch`.
//! Invariant: every command that reaches Vibe is planned as `Session` or `Forward`, and both are
//! executed only through the gate; `Local` commands use the core's dedicated handlers.

use serde_json::{Map, Value, json};

use crate::command::Command;

/// What executing a command means.
pub(crate) enum Planned {
    /// A gated call whose result is a session state (loaded into the store, answered with the
    /// snapshot).
    Session {
        /// Method.
        method: &'static str,
        /// Params.
        params: Value,
    },
    /// A gated call answered with the server's result.
    Forward {
        /// Method (owned: `call` takes any name).
        method: String,
        /// Params.
        params: Value,
        /// Confirmation token text, when given.
        token: Option<String>,
        /// Only the typed composer access switch may use the trusted-window Full access exception.
        trusted_access_switch: bool,
    },
    /// Handled by the core itself (reads, selection, callbacks, confirmations, listing).
    Local(Command),
}

/// Builds a params object from optional fields, leaving out the absent ones.
fn object(fields: Vec<(&str, Option<Value>)>) -> Value {
    let map: Map<String, Value> = fields
        .into_iter()
        .filter_map(|(k, v)| v.map(|v| (k.to_owned(), v)))
        .collect();
    Value::Object(map)
}

fn session(method: &'static str, params: Value) -> Planned {
    Planned::Session { method, params }
}

fn forward(method: &str, params: Value, token: Option<String>) -> Planned {
    Planned::Forward {
        method: method.to_owned(),
        params,
        token,
        trusted_access_switch: false,
    }
}

/// Plans one command.
pub(crate) fn plan(command: Command) -> Planned {
    match command {
        Command::RefreshSession { session_id } => {
            session("session/read", json!({"sessionId": session_id}))
        }
        Command::StartSession { agent_config } => {
            session("session/start", object(vec![("agentConfig", agent_config)]))
        }
        Command::ResumeSession {
            session_id,
            agent_config,
        } => session(
            "session/resume",
            object(vec![
                ("sessionId", Some(Value::String(session_id))),
                ("agentConfig", agent_config),
            ]),
        ),
        Command::ForkSession {
            source_session_id,
            entry_id,
            after_turn_id,
            agent_config,
        } => session(
            "session/fork",
            object(vec![
                ("sourceSessionId", Some(Value::String(source_session_id))),
                ("entryId", entry_id.map(Value::String)),
                ("afterTurnId", after_turn_id.map(Value::String)),
                ("agentConfig", agent_config),
            ]),
        ),
        Command::RewindSession {
            session_id,
            entry_id,
            restore_files,
            inplace,
        } => session(
            "session/rewind",
            object(vec![
                ("sessionId", Some(Value::String(session_id))),
                ("entryId", Some(Value::String(entry_id))),
                ("restoreFiles", restore_files.map(Value::Bool)),
                ("inplace", inplace.map(Value::Bool)),
            ]),
        ),
        other => plan_turns_and_calls(other),
    }
}

fn plan_turns_and_calls(command: Command) -> Planned {
    match command {
        Command::StartTurn {
            session_id,
            message,
            client_user_message_id,
        } => forward(
            "turn/start",
            object(vec![
                ("sessionId", Some(Value::String(session_id))),
                ("message", Some(Value::Array(message))),
                (
                    "clientUserMessageId",
                    client_user_message_id.map(Value::String),
                ),
            ]),
            None,
        ),
        Command::SteerTurn {
            session_id,
            expected_turn_id,
            message,
            client_user_message_id,
        } => forward(
            "turn/steer",
            object(vec![
                ("sessionId", Some(Value::String(session_id))),
                ("expectedTurnId", Some(Value::String(expected_turn_id))),
                ("message", Some(Value::Array(message))),
                (
                    "clientUserMessageId",
                    client_user_message_id.map(Value::String),
                ),
            ]),
            None,
        ),
        Command::InterruptTurn {
            session_id,
            expected_turn_id,
        } => forward(
            "turn/interrupt",
            json!({"sessionId": session_id, "expectedTurnId": expected_turn_id}),
            None,
        ),
        Command::PickModel {
            session_id,
            model_alias,
            model_name,
            model_display_name,
            reasoning_effort,
        } => forward(
            "config/model/write",
            object(vec![
                ("sessionId", Some(Value::String(session_id))),
                ("sessionOnly", Some(Value::Bool(true))),
                ("modelAlias", model_alias.map(Value::String)),
                ("_modelName", model_name.map(Value::String)),
                ("_modelDisplayName", model_display_name.map(Value::String)),
                ("reasoningEffort", reasoning_effort.map(Value::String)),
            ]),
            None,
        ),
        Command::SetAccessMode {
            session_id,
            agent_name,
            confirmation_token,
        } => Planned::Forward {
            method: "session/agent/update".to_owned(),
            params: json!({"sessionId": session_id, "agentName": agent_name}),
            token: confirmation_token,
            trusted_access_switch: true,
        },
        Command::Call {
            method,
            params,
            confirmation_token,
        } => forward(&method, params, confirmation_token),
        local => Planned::Local(local),
    }
}
