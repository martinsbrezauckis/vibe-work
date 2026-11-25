//! Native-dialog texts for connector and MCP server changes.
//! The Debug API acts as the user for connector changes (off/on, sign out, remove), without
//! an additional dialog.
//!
//! Depends on: `serde_json`, vw-gate (`dialog_value`). Called by: `confirmer` (dialog texts for the gate's own
//! dialogs, which the window still sees).
//! Invariants:
//! - The dialog text names the target (alias, server name, address) and what happens, in plain words; every
//!   value from the request is sanitized and quoted (`vw_gate::dialog_value`); who
//!   asks is the request's `requester` field, which the dialog shows first. The host shows only this summary.

use serde_json::Value;

/// The value of `key`, sanitized and quoted for a dialog sentence (`vw_gate::dialog_value`): a name cannot
/// add lines, reorder the text or close its quotes.
fn quoted(params: &Value, key: &str) -> String {
    let raw = params.get(key).and_then(Value::as_str).unwrap_or("?");
    vw_gate::dialog_quote(raw)
}

fn off(params: &Value) -> bool {
    params.get("disabled").and_then(Value::as_bool) == Some(true)
}

/// The dialog sentence for adding a server that needs no sign-in (`connectors.addNoAuthServer`): what is
/// connected, that no credential is sent, and what is true about the address.
pub(crate) fn no_auth_summary(name: &str, url: &str) -> String {
    let scope = vw_gate::local_server_disclosure(url).unwrap_or_else(|| {
        format!(
            "Anything you or the agent send to this server leaves this computer. {}",
            vw_gate::MCP_NOT_VOUCHED
        )
    });
    format!(
        "Add the MCP server {} at {} without a sign-in. Vibe will connect to it in your chats and send it no key \
         or token. {scope}",
        vw_gate::dialog_quote(name),
        vw_gate::dialog_quote(url)
    )
}

/// What the dialog for a command server shows (`connectors.addCommandServer` / `updateCommandServer`).
pub(crate) struct CommandDialog<'a> {
    /// True: a change of an existing server; false: a new one.
    pub change: bool,
    /// True: turn an existing disabled command server on without editing it.
    pub enable: bool,
    /// True when an imported program is being reviewed in the import dialog.
    pub import: bool,
    pub name: &'a str,
    /// The resolved absolute program.
    pub program: &'a str,
    pub args: &'a [String],
    pub cwd: Option<&'a str>,
    /// Name and plain value; `None`: the value looks like a key and stays hidden.
    pub env: &'a [(String, Option<String>)],
}

/// The dialog text for a command server: the name, the resolved program, each argument, the working folder, the variable
/// names (plain values shown, hidden ones not), and plainly what it means. Every value is quoted in full
/// (`vw_gate::dialog_quote`), so nothing is cut or reordered.
pub(crate) fn command_server_summary(dialog: &CommandDialog<'_>) -> String {
    let quote = vw_gate::dialog_quote;
    let list = |items: Vec<String>| {
        if items.is_empty() {
            "(none)".to_owned()
        } else {
            items.join(", ")
        }
    };
    let args = list(dialog.args.iter().map(|a| quote(a)).collect());
    let env = list(
        dialog
            .env
            .iter()
            .map(|(name, value)| match value {
                Some(value) => format!("{} = {}", quote(name), quote(value)),
                None if dialog.import => quote(name),
                None => format!("{} (value hidden: it looks like a key)", quote(name)),
            })
            .collect(),
    );
    let folder = dialog
        .cwd
        .map_or_else(|| "(none: Vibe's own folder)".to_owned(), quote);
    let action = if dialog.import {
        "Import the command MCP server"
    } else if dialog.enable {
        vw_vocabulary::texts::COMMAND_SERVER_ENABLE_CONFIRM_ACTION
    } else if dialog.change {
        "Save changes to the command MCP server"
    } else {
        "Add the command MCP server"
    };
    format!(
        "{action} {}.\nProgram: {}\nArguments: {args}\nWorking folder: {folder}\nEnvironment variables: {env}\n\
         This program runs on this computer with your account's rights. Its tools can reach what your account can reach \
         here: files, programs and local services. Environment values are saved as plain text in Vibe's settings file. {} \
         You decide.",
        quote(dialog.name),
        quote(dialog.program),
        vw_gate::MCP_NOT_VOUCHED
    )
}

/// The dialog sentence for a connector or MCP server call, `None` for other methods. Who asks is not
/// part of it: the dialog leads with the request's `requester` (`vw_host::dialog_text`).
pub(crate) fn describe(method: &str, params: &Value) -> Option<String> {
    let tool = params
        .get("toolName")
        .and_then(Value::as_str)
        .map(|t| format!(" (only its tool {})", vw_gate::dialog_quote(t)))
        .unwrap_or_default();
    let summary = match method {
        "connector_catalog/toggle" => {
            let alias = quoted(params, "alias");
            if off(params) {
                format!(
                    "Turn off the Mistral connector {alias}{tool} in Vibe: chats stop using it."
                )
            } else {
                format!(
                    "Turn on the Mistral connector {alias}{tool} in Vibe: chats may use it (in each chat's access mode)."
                )
            }
        }
        "mcp_catalog/toggle" | "mcp/toggle" => {
            let name = quoted(params, "name");
            if off(params) {
                format!("Turn off the MCP server {name}{tool}: chats stop using it.")
            } else {
                format!("Turn on the MCP server {name}{tool}: chats may use its tools.")
            }
        }
        "mcp_catalog/logout" | "mcp/logout" => format!(
            "Sign out of the MCP server {}: Vibe deletes its sign-in from the system keychain.",
            quoted(params, "name")
        ),
        "mcp_catalog/remove" => format!(
            "Remove the MCP server {} from Vibe's config and delete its sign-in.",
            quoted(params, "name")
        ),
        "mcp_catalog/add" | "mcp/add" => {
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .map(|n| format!(" as {}", vw_gate::dialog_quote(n)))
                .unwrap_or_default();
            let scopes = params.get("scopes").and_then(Value::as_array).map_or_else(
                || "No sign-in scopes were requested.".to_owned(),
                |scopes| {
                    format!(
                        "Requested sign-in scopes: {}.",
                        scopes
                            .iter()
                            .filter_map(Value::as_str)
                            .map(vw_gate::dialog_quote)
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                },
            );
            let transport = params.get("transport").and_then(Value::as_str).map_or_else(
                || "The server chooses its default connection mode.".to_owned(),
                |mode| format!("Connection mode: {}.", vw_gate::dialog_quote(mode)),
            );
            let insecure = params
                .get("url")
                .and_then(Value::as_str)
                .and_then(vw_gate::local_server_disclosure)
                .unwrap_or_else(|| {
                    let transport_risk = if params.get("allowInsecureHttp").and_then(Value::as_bool)
                        == Some(true)
                    {
                        "Unencrypted HTTP is allowed: traffic to this server can be read or changed on the network."
                    } else {
                        "Unencrypted HTTP is not allowed."
                    };
                    format!("{transport_risk} {}", vw_gate::MCP_NOT_VOUCHED)
                });
            format!(
                "Add the MCP server at {}{name} to Vibe's config. Vibe will connect to it in your chats; if it asks for a sign-in, the sign-in page opens in your browser and the token stays in the system keychain. {transport} {scopes} {insecure}",
                quoted(params, "url"),
            )
        }
        "mcp_catalog/login" | "mcp/login" => format!(
            "Start the sign-in for the MCP server {}. Vibe asks the server for its sign-in page; you finish it in your browser.",
            quoted(params, "name")
        ),
        "connector_catalog/connect" => format!(
            "Connect the Mistral connector {}. It needs no sign-in; Mistral keeps the connection on your account.",
            quoted(params, "alias")
        ),
        "connector_catalog/auth/request" => format!(
            "Start the sign-in for the Mistral connector {}. Mistral gives the vendor's sign-in page; you finish it in your browser and the account link is kept by Mistral.",
            quoted(params, "alias")
        ),
        _ => return None,
    };
    let Some(id) = params.get("sessionId").and_then(Value::as_str) else {
        return Some(summary);
    };
    Some(format!("Session {}. {summary}", vw_gate::dialog_quote(id)))
}

#[cfg(test)]
mod kg1_tests {
    use serde_json::json;

    use super::describe;

    #[test]
    fn mcp_add_summary_names_full_address_and_session() {
        let address = format!("https://example.test/{}", "route/".repeat(100));
        let text = describe(
            "mcp_catalog/add",
            &json!({"sessionId": "s1", "url": address, "name": "server", "scopes": ["mail.read", "files.write"], "transport": "streamable-http", "allowInsecureHttp": true}),
        )
        .unwrap();
        assert!(text.contains(&address));
        assert!(text.contains("Session \"s1\""));
        assert!(text.contains("server"));
        assert!(text.contains("mail.read") && text.contains("files.write"));
        assert!(text.contains("streamable-http") && text.contains("Unencrypted HTTP is allowed"));
        assert!(!text.contains("mcp_catalog/add"));
    }
}
