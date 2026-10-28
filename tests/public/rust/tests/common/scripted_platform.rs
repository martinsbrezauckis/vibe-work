//! Scripted Mistral connector HTTP and an OAuth MCP service for real-Vibe tests. No Vibe session/protocol methods.
//! Depends on the pinned bootstrap schema, MCP streamable HTTP and OAuth endpoints. Called by scripted_model.
use std::collections::BTreeSet;

use axum::Json;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

#[derive(Default)]
pub struct Platform {
    pub base: String,
    connected: BTreeSet<String>,
}

const ALIASES: &[&str] = &[
    "atlassian",
    "notion",
    "box",
    "stripe",
    "linear",
    "slack",
    "github_app",
    "outlook_calendar",
    "web_search",
    "gmail",
    "outlook",
    "google_calendar",
    "sharepoint_mcp",
    "bigquery",
    "google_drive_mcp",
    "sharepoint_online",
];
pub const TOKEN: &str = "vwfaketokenTESTSECRET";

impl Platform {
    pub fn response(
        &mut self,
        method: &Method,
        uri: &Uri,
        headers: &HeaderMap,
        body: &Value,
    ) -> Option<Response> {
        let path = uri.path();
        let query: std::collections::BTreeMap<_, _> =
            url::form_urlencoded::parse(uri.query().unwrap_or("").as_bytes())
                .into_owned()
                .collect();
        if method == Method::GET && path == "/v1/users/me" {
            return Some(Json(json!({"id":"rv1-user","organization":{"id":"rv1-org","name":"Scripted organization"},"workspace":{"id":"rv1-workspace","name":"Scripted workspace"}})).into_response());
        }
        if method == Method::GET && path == "/v1/connectors/bootstrap" {
            let connectors: Vec<_> = ALIASES.iter().map(|alias| {
                let ready = *alias == "web_search" || self.connected.contains(*alias);
                let tools = if *alias == "web_search" { vec!["web_search", "news_search", "open", "find", "image_search"] }
                    else if ready { vec!["list_issues"] } else { vec![] };
                json!({"id":alias, "name":alias, "display_name":alias, "protocol":"mcp",
                    "status":{"is_ready":ready}, "auth_action":{"type":if *alias == "sharepoint_online" {"credentials_setup"} else if *alias == "bigquery" {"none"} else {"oauth"}},
                    "bootstrap_errors": if *alias == "bigquery" {json!(["disabled: turned off by the scripted platform"])} else {json!([])},
                    "tools":tools.into_iter().map(tool).collect::<Vec<_>>()})
            }).collect();
            return Some(Json(json!({"connectors":connectors})).into_response());
        }
        if method == Method::GET
            && path.starts_with("/v1/connectors/")
            && path.ends_with("/auth_url")
        {
            let alias = path
                .trim_start_matches("/v1/connectors/")
                .trim_end_matches("/auth_url");
            if alias == "sharepoint_mcp" {
                return Some(
                    (StatusCode::SERVICE_UNAVAILABLE, "scripted auth URL failure").into_response(),
                );
            }
            return Some(Json(json!({"auth_url":format!("{}/authorize?connector={alias}", self.base),"ttl":300})).into_response());
        }
        if method == Method::GET && path == "/authorize" {
            if let Some(alias) = query.get("connector") {
                self.connected.insert(alias.clone());
                return Some(
                    (
                        [("x-scripted-token", TOKEN)],
                        "Signed in to the scripted connector",
                    )
                        .into_response(),
                );
            }
            let mut location = url::Url::parse(query.get("redirect_uri")?).ok()?;
            location
                .query_pairs_mut()
                .append_pair("code", "scripted-code")
                .append_pair("state", query.get("state").map_or("", String::as_str));
            return Some((StatusCode::FOUND, [("location", location.to_string())]).into_response());
        }
        if path.starts_with("/.well-known/oauth-protected-resource") {
            return Some(Json(json!({"resource":format!("{}/mcp",self.base),"authorization_servers":[self.base],"scopes_supported":["notes"]})).into_response());
        }
        if path == "/.well-known/oauth-authorization-server"
            || path == "/.well-known/openid-configuration"
        {
            return Some(Json(json!({"issuer":self.base,"authorization_endpoint":format!("{}/authorize?server=notes",self.base),
                "token_endpoint":format!("{}/token",self.base),"registration_endpoint":format!("{}/register",self.base),
                "response_types_supported":["code"],"grant_types_supported":["authorization_code"],
                "code_challenge_methods_supported":["S256"],"token_endpoint_auth_methods_supported":["none"]})).into_response());
        }
        if path == "/register" {
            let mut reply = body.clone();
            reply["client_id"] = json!("scripted-notes-client");
            return Some((StatusCode::CREATED, Json(reply)).into_response());
        }
        if path == "/token" {
            return Some(Json(json!({"access_token":TOKEN,"token_type":"Bearer","expires_in":3600,"scope":"notes"})).into_response());
        }
        if path == "/mcp"
            || path == "/noauth/mcp"
            || path == "/notes/mcp"
            || path == "/notes-extra/mcp"
            || path.starts_with("/fixture/")
            || path.starts_with("/v1/connectors-gateway/")
        {
            if path == "/mcp"
                && headers.get("authorization").and_then(|v| v.to_str().ok())
                    != Some(&format!("Bearer {TOKEN}"))
            {
                return Some((StatusCode::UNAUTHORIZED, [("www-authenticate", format!("Bearer resource_metadata=\"{}/.well-known/oauth-protected-resource/mcp\"",self.base))]).into_response());
            }
            if method == Method::GET {
                return Some(StatusCode::METHOD_NOT_ALLOWED.into_response());
            }
            if method == Method::DELETE {
                return Some(StatusCode::OK.into_response());
            }
            let Some(id) = body.get("id") else {
                return Some(StatusCode::ACCEPTED.into_response());
            };
            let result = match body["method"].as_str().unwrap_or("") {
                "initialize" => {
                    json!({"protocolVersion":"2025-03-26","serverInfo":{"name":"scripted-platform","version":"1"},"capabilities":{"tools":{}}})
                }
                "tools/list" => {
                    if path.starts_with("/fixture/") {
                        json!({"tools":[tool("browser_snapshot"),tool("search"),tool("chrome_page_read"),tool("chrome_page_commit")]})
                    } else if matches!(path, "/notes/mcp" | "/notes-extra/mcp") {
                        json!({"tools":[tool("list_notes"),tool("add_note")]})
                    } else {
                        json!({"tools":[tool(if path == "/mcp" {"list_notes"} else {"web_search"})]})
                    }
                }
                "tools/call" if path != "/mcp" => {
                    let refs = search_refs();
                    json!({"content":[{"type":"text","text":refs.to_string()}],"structuredContent":refs,"isError":false})
                }
                "tools/call" => {
                    json!({"content":[{"type":"text","text":"One scripted note"}],"isError":false})
                }
                _ => json!({}),
            };
            return Some(Json(json!({"jsonrpc":"2.0","id":id,"result":result})).into_response());
        }
        None
    }
}

fn tool(name: &str) -> Value {
    json!({"name":name,"description":format!("Scripted {name}"),"inputSchema":{"type":"object","properties":{"query":{"type":"string"}}}})
}

fn search_refs() -> Value {
    let sources = [
        (
            "Harbour history - City archive",
            "https://archive.example.org/harbour/1887?ref=vw",
        ),
        (
            "bank.example (looks like a bank)",
            "https://bank.example@evil.example/login",
        ),
        ("Müller Hafen", "https://müller.example/hafen"),
        ("<img src=x onerror=alert(1)>", "javascript:alert(1)"),
    ];
    Value::Object(
        sources
            .iter()
            .enumerate()
            .map(|(rank, (title, url))| {
                (
                    format!("ref{rank}"),
                    json!({"title":title,"url":url,"rank":rank,"snippets":["A scripted source."]}),
                )
            })
            .collect(),
    )
}
