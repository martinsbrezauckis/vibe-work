//! Complete plain-language text for gate-owned native confirmations.
//!
//! Depends on: dialog sanitizer and validated gate params. Called by: gate before asking its confirmer.
//! The method and params bind the token but are never displayed by the native host.

use serde_json::Value;

use crate::dialog::dialog_quote;

fn named(params: &Value, key: &str, label: &str, access_mode: bool) -> Option<String> {
    params
        .get(key)
        .and_then(|value| match value {
            Value::String(text) if !text.is_empty() => Some(text.clone()),
            Value::Number(number) => Some(number.to_string()),
            _ => None,
        })
        .map(|value| {
            let display = if access_mode {
                vw_vocabulary::label("accessModes", &value)
            } else {
                &value
            };
            format!("{label} {}", dialog_quote(display))
        })
}

fn plain_value(value: &Value) -> String {
    match value {
        Value::Null => "nothing".to_owned(),
        Value::Bool(on) => if *on { "on" } else { "off" }.to_owned(),
        Value::Number(number) => number.to_string(),
        Value::String(text) => dialog_quote(text),
        Value::Array(items) => items.iter().map(plain_value).collect::<Vec<_>>().join("; "),
        Value::Object(fields) => fields
            .iter()
            .map(|(name, value)| format!("{}: {}", dialog_quote(name), plain_value(value)))
            .collect::<Vec<_>>()
            .join("; "),
    }
}

/// Plain action for each confirmed method, or the gate rule's specific description.
fn action_sentence<'a>(method: &str, params: &Value, base: &'a str) -> &'a str {
    match method {
        "agents/install" => "Install an agent that can run code in Vibe Work?",
        "agents/uninstall" => "Uninstall an agent from Vibe Work?",
        // Name the certificate only when this write sets one (SSL_CERT_FILE / SSL_CERT_DIR with a value).
        "config/proxy/write"
            if ["SSL_CERT_FILE", "SSL_CERT_DIR"].iter().any(|key| {
                params["changes"][key]
                    .as_str()
                    .is_some_and(|v| !v.is_empty())
            }) =>
        {
            "Change how Vibe connects to the internet? Its requests may go through the proxy you entered, and a certificate you chose will be trusted."
        }
        "config/proxy/write" => {
            "Change how Vibe connects to the internet? Its requests may go through the proxy you entered."
        }
        "connector_catalog/auth/request" => "Open the sign-in page for this Mistral connector?",
        "connector_catalog/connect" => "Connect this Mistral connector?",
        "mcp/add" | "mcp_catalog/add" => {
            "Add this MCP server? Vibe can connect to it from sessions, and its tools may run code or send data."
        }
        "mcp/login" | "mcp_catalog/login" => "Open the sign-in page for this MCP server?",
        "projectLinks/inspectRoot" => "Inspect this project root for a link?",
        "projectLinks/list" => "List project links?",
        "projectLinks/resolveRoot" => "Resolve this project-link root?",
        "projectLinks/save" => "Save this project link?",
        "projectLinks/unlink" => "Unlink this project location?",
        "skills/convertLocal" => {
            "Convert a local skill for use in Vibe? It may provide instructions or code to an agent."
        }
        "skills/import" => {
            "Import this skill into Vibe? It may provide instructions or code to an agent."
        }
        "skills/setLatest" => "Update this skill to its latest version?",
        "skills/setVersion" => "Set this skill to the requested version?",
        _ => base,
    }
}

/// Adds one line per settings change. The privacy switches and the default access mode read as the Settings page
/// names them; any other key keeps its config path (every change of the write stays visible: only the last
/// escalation is the dialog's sentence).
fn append_config_changes(params: &Value, parts: &mut Vec<String>) {
    let Some(ops) = params.get("ops").and_then(Value::as_array) else {
        return;
    };
    for op in ops {
        let (Some(path), Some(kind)) = (
            op.get("path").and_then(Value::as_str),
            op.get("op").and_then(Value::as_str),
        ) else {
            continue;
        };
        let named = match path {
            "/enable_telemetry" => Some(("Send usage data to Mistral", "on")),
            "/experiments/enable" => Some(("Take part in Mistral's experiments", "on")),
            "/default_agent" => Some(("Default access mode", "")),
            _ => None,
        };
        let value = op.get("value");
        let line = match (named, kind, value) {
            (Some((label, default)), "remove", _) if !default.is_empty() => {
                format!("{label}: back to the default ({default})")
            }
            (Some((label, _)), "remove", _) => format!("{label}: back to the default"),
            (Some((label, _)), _, Some(value)) => {
                let display = if path == "/default_agent" {
                    value
                        .as_str()
                        .map(|id| dialog_quote(vw_vocabulary::label("accessModes", id)))
                } else {
                    None
                }
                .unwrap_or_else(|| plain_value(value));
                format!("{label}: {display}")
            }
            (_, _, value) => {
                let value = value.map_or_else(String::new, |v| format!(" to {}", plain_value(v)));
                format!("{kind} setting {}{value}", dialog_quote(path))
            }
        };
        parts.push(line);
    }
}

/// One line of a `config/proxy/write` dialog: what the setting does, in plain words. A proxy address is shown as
/// `scheme://host:port` only (the dialog and the logs must not carry a sign-in); the token still binds the real value.
fn proxy_change(key: &str, value: Option<&str>) -> String {
    let (label, action) = match key {
        "HTTP_PROXY" => ("Web (HTTP) proxy", "send requests through"),
        "HTTPS_PROXY" => ("Secure web (HTTPS) proxy", "send requests through"),
        "ALL_PROXY" => ("Proxy for all other requests", "send requests through"),
        "NO_PROXY" => ("Requests that skip the proxy", "skip the proxy for"),
        "SSL_CERT_FILE" => (
            "Certificate file",
            "also trust the certificates in the file",
        ),
        "SSL_CERT_DIR" => (
            "Certificate folder",
            "also trust the certificates in the folder",
        ),
        _ => (key, "change to"),
    };
    match value {
        None => format!("{label}: remove this setting"),
        // An address is shown as the scheme, host and port the client will connect to, and nothing else (the sign-in
        // is only named); the other settings are shown as they are.
        Some(text) if matches!(key, "HTTP_PROXY" | "HTTPS_PROXY" | "ALL_PROXY") => format!(
            "{label}: {action} {}{}",
            dialog_quote(&vw_mistral::network::mask_proxy_url(text)),
            if vw_mistral::network::has_sign_in(text) {
                " with a user name and password"
            } else {
                ""
            }
        ),
        Some(text) => format!("{label}: {action} {}", dialog_quote(text)),
    }
}

/// The gate's complete visible sentence. Values are selected by meaning, never dumped as JSON.
pub(crate) fn confirmation_summary(method: &str, params: &Value, base: &str) -> String {
    let mut parts = vec![action_sentence(method, params, base).to_owned()];
    for (key, label) in [
        ("sessionId", vw_vocabulary::label("workNames", "session")),
        (
            "agentName",
            if method == "session/agent/update" {
                vw_vocabulary::text("accessMode").unwrap_or_default()
            } else {
                "Agent"
            },
        ),
        ("agent", "Agent"),
        ("name", "Name"),
        ("alias", "Alias"),
        ("url", "Server address"),
        ("rootPath", "Project folder"),
        ("cwd", "Folder"),
        ("skillId", "Skill"),
        ("projectId", vw_vocabulary::label("workNames", "projectTab")),
        ("projectName", "Project name"),
        ("expectedGithubRepoUrl", "Expected GitHub repository"),
        ("pickerId", "Picker"),
        ("operationId", "Operation"),
        ("defaultBranch", "Default branch"),
        ("version", "Version"),
        ("scope", "Scope"),
        ("cursor", "List position"),
        ("purpose", "Purpose"),
        ("prompt", "Prompt sent to Vibe Code"),
    ] {
        // The session of a proxy or settings change is a helper chat the user never saw: its id would only confuse.
        if matches!(method, "config/proxy/write" | "config/write") && key == "sessionId" {
            continue;
        }
        if let Some(value) = named(
            params,
            key,
            label,
            method == "session/agent/update" && key == "agentName",
        ) {
            parts.push(value);
        }
    }
    if matches!(method, "mcp/add" | "mcp_catalog/add") {
        if let Some(scopes) = params.get("scopes").and_then(Value::as_array) {
            parts.push(format!(
                "Requested sign-in scopes: {}",
                scopes
                    .iter()
                    .filter_map(Value::as_str)
                    .map(dialog_quote)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if let Some(mode) = named(params, "transport", "Connection mode", false) {
            parts.push(mode);
        }
        parts.push(
            if params.get("allowInsecureHttp").and_then(Value::as_bool) == Some(true) {
                "Unencrypted HTTP is allowed; network traffic may be read or changed.".to_owned()
            } else {
                "Unencrypted HTTP is not allowed.".to_owned()
            },
        );
    }
    if let Some(config) = params.get("agentConfig") {
        if let Some(name) = named(
            config,
            "agent",
            vw_vocabulary::text("accessMode").unwrap_or_default(),
            true,
        ) {
            parts.push(name);
        }
        if config.get("autoApprove").and_then(Value::as_bool) == Some(true) {
            parts.push(format!(
                "{} is on: the agent will run tools without asking.",
                vw_vocabulary::label("accessModes", "auto-approve")
            ));
        }
    }
    if method == "config/proxy/write"
        && let Some(changes) = params.get("changes").and_then(Value::as_object)
    {
        for (key, value) in changes {
            parts.push(proxy_change(
                key,
                value.as_str().filter(|text| !text.is_empty()),
            ));
        }
    }
    if method == "config/write" {
        append_config_changes(params, &mut parts);
    }
    if method == "session/shellCommand"
        && let Some(command) = params.get("command").and_then(Value::as_str)
    {
        parts.push(format!("Command:\n{}", dialog_quote(command)));
    }
    parts.join("\n\n")
}
