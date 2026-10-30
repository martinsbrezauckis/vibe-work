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
        "config/proxy/write" => {
            "Change how Vibe connects to the internet? Its requests may go through the proxy you entered, and a certificate you chose will be trusted."
        }
        "connector_catalog/auth/request" => "Open the sign-in page for this Mistral connector?",
        "connector_catalog/connect" => "Connect this Mistral connector?",
        "mcp/add" | "mcp_catalog/add" => {
            "Add this MCP server? Vibe can connect to it from sessions, and its tools may run code or send data."
        }
        "mcp/login" | "mcp_catalog/login" => "Open the sign-in page for this MCP server?",
        "projectLinks/create" => "Create a project link for this folder?",
        "projectLinks/inspectRoot" => "Inspect this project root for a link?",
        "projectLinks/link" => "Link this project to another location?",
        "projectLinks/list" => "List project links?",
        "projectLinks/picker/load" | "projectLinks/picker/loadMore" => {
            "Load folders for the project-link picker?"
        }
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
        "vibeCode/projects/cancel" => "Cancel this Vibe Code project operation?",
        "vibeCode/projects/create" => {
            "Create this Vibe Code project? Project data may be sent to Mistral."
        }
        "vibeCode/projects/loadMore" => "Load more Vibe Code projects?",
        "vibeCode/projects/open" => "Open this Vibe Code project?",
        "vibeCode/projects/recover" => "Recover this Vibe Code project?",
        "vibeCode/projects/select" => "Select this Vibe Code project?",
        "vibeCode/projects/unlink" => "Unlink this Vibe Code project?",
        "vibeCode/teleport/cancel" => "Cancel this Vibe Code transfer?",
        "vibeCode/teleport/push/respond" => {
            if params.get("approved").and_then(Value::as_bool) == Some(true) {
                "Approve this Vibe Code transfer? Project data may leave this machine."
            } else {
                "Decline this Vibe Code transfer request?"
            }
        }
        "vibeCode/teleport/start" => {
            "Start a Vibe Code transfer? Project data may leave this machine."
        }
        _ => base,
    }
}

/// Adds settings changes, translating access-profile values only for their known setting.
fn append_config_changes(params: &Value, parts: &mut Vec<String>) {
    let Some(ops) = params.get("ops").and_then(Value::as_array) else {
        return;
    };
    for op in ops {
        if let (Some(path), Some(kind)) = (
            op.get("path").and_then(Value::as_str),
            op.get("op").and_then(Value::as_str),
        ) {
            let value = op.get("value").map_or_else(String::new, |value| {
                let display = if path == "/default_agent" {
                    value
                        .as_str()
                        .map(|id| dialog_quote(vw_vocabulary::label("accessModes", id)))
                } else {
                    None
                }
                .unwrap_or_else(|| plain_value(value));
                format!(" to {display}")
            });
            parts.push(format!("{kind} setting {}{value}", dialog_quote(path)));
        }
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
        // The session of a proxy change is a helper chat the user never saw: its id would only confuse.
        if method == "config/proxy/write" && key == "sessionId" {
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::class::Class;
    use crate::table::classified_methods;

    use super::confirmation_summary;

    #[test]
    fn shell_command_is_complete_and_not_clipped() {
        let command = format!("printf '%s' {}", "long-argument-".repeat(100));
        let summary = confirmation_summary(
            "session/shellCommand",
            &json!({"sessionId": "s1", "cwd": "/work", "command": command}),
            "Run this shell command now",
        );
        assert!(summary.contains("Session \"s1\""));
        assert!(summary.contains("Folder \"/work\""));
        assert!(summary.contains(&format!(
            "Command:\n{}",
            crate::dialog::dialog_quote(&command)
        )));
        assert!(!summary.contains("session/shellCommand"));
    }

    #[test]
    fn confirmed_actions_use_plain_names_without_dumping_params() {
        let summary = confirmation_summary(
            "skills/import",
            &json!({"sessionId": "s1", "skillId": "sk-1", "opaque": {"raw": true}}),
            "generic",
        );
        assert!(summary.contains("Import this skill"));
        assert!(summary.contains("Skill \"sk-1\""));
        assert!(!summary.contains("skills/import"));
        assert!(!summary.contains("opaque"));
        assert!(!summary.contains('{'));
    }

    #[test]
    fn every_confirmed_method_has_a_plain_action_sentence() {
        for (method, class) in classified_methods() {
            if class != Class::Confirmed {
                continue;
            }
            let text = confirmation_summary(method, &json!({}), "UNMAPPED");
            assert!(
                !text.starts_with("UNMAPPED"),
                "{method} has no action sentence"
            );
        }
    }

    #[test]
    fn proxy_dialog_hides_the_sign_in_and_says_what_each_setting_does() {
        let summary = confirmation_summary(
            "config/proxy/write",
            &json!({"sessionId": "s1", "changes": {
                "HTTPS_PROXY": "http://alice:s3cret@proxy.corp:8080",
                "NO_PROXY": ".corp,10.0.0.0/8",
                "SSL_CERT_FILE": "/etc/ssl/company-ca.pem",
                "HTTP_PROXY": null
            }}),
            "generic",
        );
        assert!(
            !summary.contains("s3cret") && !summary.contains("alice"),
            "{summary}"
        );
        for line in [
            "Secure web (HTTPS) proxy: send requests through \"http://proxy.corp:8080\" with a user name and password",
            "Requests that skip the proxy: skip the proxy for \".corp,10.0.0.0/8\"",
            "Certificate file: also trust the certificates in the file \"/etc/ssl/company-ca.pem\"",
            "Web (HTTP) proxy: remove this setting",
        ] {
            assert!(summary.contains(line), "{line}\n{summary}");
        }
        assert!(!summary.contains("config/proxy/write") && !summary.contains('{'));
    }

    #[test]
    fn the_dialog_shows_the_host_the_client_connects_to_whatever_the_text_looks_like() {
        // `#@` makes a text that looks like it ends in proxy.corp; the client connects to evil.example. (The gate
        // refuses such an address first; the dialog must still never be misleading.)
        let summary = confirmation_summary(
            "config/proxy/write",
            &json!({"sessionId": "s1", "changes": {
                "HTTPS_PROXY": "http://evil.example:3128#@proxy.corp:8080"}}),
            "generic",
        );
        assert!(
            summary.contains("\"http://evil.example:3128\""),
            "{summary}"
        );
        assert!(!summary.contains("proxy.corp"), "{summary}");
    }

    #[test]
    fn transfer_decision_and_prompt_are_visible() {
        let accepted = confirmation_summary(
            "vibeCode/teleport/push/respond",
            &json!({"approved": true, "operationId": "op-1"}),
            "generic",
        );
        let declined = confirmation_summary(
            "vibeCode/teleport/push/respond",
            &json!({"approved": false, "operationId": "op-1"}),
            "generic",
        );
        assert!(accepted.contains("Approve this Vibe Code transfer"));
        assert!(declined.contains("Decline this Vibe Code transfer"));
        let prompt = "Send these  two lines\nwith this quote \"";
        let started = confirmation_summary(
            "vibeCode/teleport/start",
            &json!({"sessionId": "s1", "projectId": "p1", "prompt": prompt}),
            "generic",
        );
        assert!(started.contains(&crate::dialog::dialog_quote(prompt)));
    }
}
