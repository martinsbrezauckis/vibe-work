//! Synthetic connector and MCP responses for core integration tests.
//! - The catalog has 16 aliases; `web_search` is ready with its 5 tools; two synthetic
//!   states for coverage: `bigquery` unavailable with a diagnostic, `sharepoint_online` needs setup;
//! - a sessionless `connector_catalog/read` answers `not_loaded` until a `connector_catalog/refresh` (the fixture);
//! - `connector_catalog/auth/request` needs the session on this channel (the fixture); it answers a request id, then
//!   sends `authRequired` and `authUrl` (or `authFailed` for `sharepoint_mcp`, synthetic);
//! - the sign-in pages are on a small HTTP listener of this process (`VW_FAKE_SIGNIN_PORT` fixes its port,
//!   else a free one): `GET /authorize?connector=<alias>` signs that connector in (the next refresh shows
//!   it ready, like Mistral's callback), `GET /authorize?server=<name>` completes that server's pending
//!   login; the answer is a small page with a marker access token in a header the client never sees;
//! - `mcp_catalog/add|remove|toggle` keep the servers in `$VIBE_HOME/config.toml` (`mcp_servers`, the layout
//!   Vibe writes); `mcp_catalog/login` blocks until the browser step (`login_blocking`, max 60 s), servers
//!   whose name starts with `noauth` fail like the fixture; `mcp_catalog/toggle` with `source: connector` is
//!   `not_implemented` (the fixture);
//! - `mcp_catalog/read` lists the configured servers (signed in: `connected` with tool `list_notes`),
//!   the connectors as `kind: connector`, and one plugin server.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::Duration;

use serde_json::{Value, json};

use crate::script::Script;

/// The methods this module answers.
pub const HANDLES: &[&str] = &[
    "connector_catalog/read",
    "connector_catalog/refresh",
    "connector_catalog/toggle",
    "connector_catalog/auth/request",
    "mcp_catalog/read",
    "mcp_catalog/refresh",
    "mcp_catalog/add",
    "mcp_catalog/logout",
    "mcp_catalog/remove",
    "mcp_catalog/toggle",
];

const ALIASES: &[(&str, &str)] = &[
    ("atlassian", "Atlassian"),
    ("notion", "Notion"),
    ("box", "Box"),
    ("stripe", "Stripe"),
    ("linear", "Linear"),
    ("slack", "slack"),
    ("github_app", "GitHub"),
    ("outlook_calendar", "outlook_calendar"),
    ("web_search", "web_search"),
    ("gmail", "gmail"),
    ("outlook", "outlook"),
    ("google_calendar", "google_calendar"),
    ("sharepoint_mcp", "SharePoint Search API"),
    ("bigquery", "BigQuery"),
    ("google_drive_mcp", "google_drive_mcp"),
    ("sharepoint_online", "SharePoint Online"),
];

/// A marker that stands for an OAuth token in the fake: it is only ever in this process and in the
/// sign-in page's response header, never in a protocol frame (the core's tests sweep for it).
pub const FAKE_TOKEN_MARKER: &str = "vwfaketokenTESTSECRET";

#[derive(Default)]
struct Server {
    url: String,
    signed_in: bool,
    disabled: bool,
}

#[derive(Default)]
struct World {
    loaded: bool,
    signed_in: BTreeSet<String>,
    disabled: BTreeSet<String>,
    servers: BTreeMap<String, Server>,
    /// Logins waiting for the browser step.
    pending_logins: BTreeSet<String>,
    next: u64,
}

struct Fake {
    world: Mutex<World>,
    done: Condvar,
    port: u16,
}

fn fake() -> &'static Fake {
    static FAKE: OnceLock<Fake> = OnceLock::new();
    FAKE.get_or_init(|| {
        let port = std::env::var("VW_FAKE_SIGNIN_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(0);
        let listener = TcpListener::bind(("127.0.0.1", port)).expect("fake sign-in listener");
        let port = listener.local_addr().expect("addr").port();
        std::thread::spawn(move || serve(listener));
        Fake {
            world: Mutex::new(World::default()),
            done: Condvar::new(),
            port,
        }
    })
}

/// Serves the sign-in pages (one request per connection; see the module docs).
fn serve(listener: TcpListener) {
    for stream in listener.incoming().flatten() {
        let mut reader = BufReader::new(&stream);
        let mut line = String::new();
        if reader.read_line(&mut line).is_err() {
            continue;
        }
        // Read the rest of the request (the headers) before answering: closing a socket with request bytes still
        // `vw_core_connectors` failed once with 10054 "forcibly closed"; reproduced on Linux as ECONNRESET).
        let mut header = String::new();
        while reader.read_line(&mut header).is_ok_and(|n| n > 0)
            && header != "\r\n"
            && header != "\n"
        {
            header.clear();
        }
        let path = line.split_whitespace().nth(1).unwrap_or("/").to_owned();
        let query = path.split_once('?').map_or("", |(_, q)| q).to_owned();
        let param = |key: &str| {
            query.split('&').find_map(|pair| {
                let (k, v) = pair.split_once('=')?;
                (k == key).then(|| v.to_owned())
            })
        };
        let f = fake();
        let mut body = "unknown sign-in".to_owned();
        {
            let mut w = f.world.lock().unwrap();
            if let Some(alias) = param("connector") {
                w.signed_in.insert(alias.clone());
                body = format!("Signed in to {alias} (fake). You can close this page.");
            } else if let Some(name) = param("server")
                && w.pending_logins.remove(&name)
            {
                if let Some(server) = w.servers.get_mut(&name) {
                    server.signed_in = true;
                }
                body = format!("Signed in to {name} (fake). You can close this page.");
            }
        }
        f.done.notify_all();
        let mut out = &stream;
        let _ = write!(
            out,
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nX-Fake-Token: {FAKE_TOKEN_MARKER}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
    }
}

fn readiness(w: &World, alias: &str) -> (&'static str, &'static str) {
    match alias {
        "web_search" => ("ready", "none"),
        "bigquery" => ("unavailable", "unknown"),
        "sharepoint_online" => ("needs_setup", "credentials_setup"),
        a if w.signed_in.contains(a) => ("ready", "oauth"),
        _ => ("needs_auth", "oauth"),
    }
}

fn tools(alias: &str) -> Value {
    match alias {
        "web_search" => json!([
            {"name": "web_search", "description": "Search the web for pages and return links, snippets, and publication dates."},
            {"name": "news_search", "description": "Search news articles with optional date and language filters."},
            {"name": "open_url", "description": "Open a web page or AFP/AP news article ID and return its content."},
            {"name": "finance_search", "description": "Search structured finance and market data."},
            {"name": "weather_search", "description": "Search structured weather data."}
        ]),
        "linear" => json!([
            {"name": "list_issues", "description": "List issues."},
            {"name": "create_issue", "description": "Create an issue."}
        ]),
        _ => json!([]),
    }
}

fn catalog_entries(w: &World) -> Vec<Value> {
    ALIASES
        .iter()
        .map(|(alias, name)| {
            let (ready, action) = readiness(w, alias);
            let tools = if ready == "ready" {
                tools(alias)
            } else {
                json!([])
            };
            let diagnostic = (*alias == "bigquery")
                .then_some("(fixture) This connector is turned off for your workspace.");
            json!({"alias": alias, "displayName": name, "readiness": ready, "authAction": action,
                "tools": tools, "diagnostic": diagnostic})
        })
        .collect()
}

fn session_sources(w: &World) -> Vec<Value> {
    ALIASES
        .iter()
        .map(|(alias, name)| {
            let (ready, _) = readiness(w, alias);
            let status = if w.disabled.contains(*alias) {
                "disabled"
            } else if ready == "ready" {
                "connected"
            } else {
                ready
            };
            let tools: Vec<Value> = tools(alias)
                .as_array()
                .into_iter()
                .flatten()
                .filter(|_| ready == "ready")
                .map(|t| json!({"name": t["name"], "description": t["description"], "enabled": true}))
                .collect();
            json!({"alias": alias, "displayName": name, "status": status, "tools": tools, "error": null})
        })
        .collect()
}

/// Writes the servers to `$VIBE_HOME/config.toml` the way Vibe does (`mcp_servers` array).
fn persist(w: &World) {
    let Some(home) = std::env::var_os("VIBE_HOME") else {
        return;
    };
    let path = std::path::Path::new(&home).join("config.toml");
    let kept: String = std::fs::read_to_string(&path)
        .unwrap_or_default()
        .lines()
        .take_while(|l| !l.starts_with("# fake mcp_servers"))
        .map(|l| format!("{l}\n"))
        .collect();
    let mut text = format!("{kept}# fake mcp_servers\nmcp_servers = [\n");
    for (name, s) in &w.servers {
        text.push_str(&format!(
            "  {{ name = {name:?}, transport = \"streamable-http\", url = {:?}, auth = {{ type = \"oauth\", scopes = [] }}, disabled = {} }},\n",
            s.url, s.disabled
        ));
    }
    text.push_str("]\n");
    let _ = std::fs::create_dir_all(&home);
    let _ = std::fs::write(path, text);
}

fn error(code: &str, message: &str) -> Value {
    json!({"__error": code, "__message": message})
}

impl Script {
    /// Answers one connector or MCP catalog request (see the module docs).
    pub fn connectors_call(self: &Arc<Self>, conn: u64, method: &str, params: &Value) -> Value {
        let f = fake();
        let sid = params["sessionId"].as_str().map(str::to_owned);
        let mut w = f.world.lock().unwrap();
        match method {
            "connector_catalog/read" => {
                let connectors = if w.loaded {
                    catalog_entries(&w)
                } else {
                    Vec::new()
                };
                let selections: Vec<Value> = w
                    .disabled
                    .iter()
                    .map(|a| json!({"alias": a, "disabled": true, "disabledTools": [], "state": "resolved"}))
                    .collect();
                let session = sid.as_ref().map(|_| {
                    json!({"acceptedCatalogRevision": "fake-rev", "acceptedSelectionRevision": "fake-sel",
                        "routeRevision": "fake-route", "sources": session_sources(&w)})
                });
                json!({"catalog": {"disposition": if w.loaded { "memory" } else { "not_loaded" },
                        "catalogRevision": if w.loaded { json!("fake-rev") } else { Value::Null },
                        "connectors": connectors},
                    "selections": selections, "session": session,
                    // What Vibe builds behind Vibe Work's key proxy (`console_base_url` = the proxy's
                    // `/console`; the core maps it back to the console.
                    "manageUrl": "http://127.0.0.1:9/console/build/connectors?shareContext=fake"})
            }
            "connector_catalog/refresh" => {
                w.loaded = true;
                json!({"catalogRevision": "fake-rev", "selectionRevision": "fake-sel", "runtime": null,
                    "pendingSelection": false})
            }
            "connector_catalog/toggle" => {
                let alias = params["alias"].as_str().unwrap_or_default().to_owned();
                if params["disabled"].as_bool() == Some(true) {
                    w.disabled.insert(alias);
                } else {
                    w.disabled.remove(&alias);
                }
                json!({"catalogRevision": "fake-rev", "selectionRevision": "fake-sel2", "runtime": null,
                    "pendingSelection": sid.is_none()})
            }
            "connector_catalog/auth/request" => {
                let sid = sid.unwrap_or_default();
                drop(w);
                if !self.holds(conn, &sid) {
                    return error("not_found", &format!("Session not found: {sid}"));
                }
                let alias = params["alias"].as_str().unwrap_or_default().to_owned();
                let mut w = f.world.lock().unwrap();
                w.next += 1;
                let request_id = format!("fake-auth-{}", w.next);
                drop(w);
                let send = self.send_to(conn);
                let (rid, s, a) = (request_id.clone(), sid.clone(), alias.clone());
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(50));
                    let Some(send) = send else { return };
                    let base = json!({"sessionId": s, "alias": a, "acceptedCatalogRevision": "fake-rev",
                        "reason": "needs_auth"});
                    send(
                        json!({"jsonrpc": "2.0", "method": "connector_catalog/authRequired", "params": base}),
                    );
                    let mut params = base.clone();
                    if a == "sharepoint_mcp" {
                        params["requestId"] = json!(rid);
                        params["code"] = json!("auth_url_unavailable");
                        send(
                            json!({"jsonrpc": "2.0", "method": "connector_catalog/authFailed", "params": params}),
                        );
                    } else {
                        params["requestId"] = json!(rid);
                        params["url"] = json!(format!(
                            "http://127.0.0.1:{}/authorize?connector={a}&state=fake-state",
                            fake().port
                        ));
                        send(
                            json!({"jsonrpc": "2.0", "method": "connector_catalog/authUrl", "params": params}),
                        );
                    }
                });
                json!({"requestId": request_id, "sessionId": sid, "alias": alias,
                    "acceptedCatalogRevision": "fake-rev"})
            }
            "mcp_catalog/read" => {
                let mut sources: Vec<Value> = w
                    .servers
                    .iter()
                    .map(|(name, s)| {
                        let status = if s.disabled {
                            "disabled"
                        } else if s.signed_in {
                            "connected"
                        } else {
                            "needs_auth"
                        };
                        let tools = if status == "connected" {
                            json!([{"name": "list_notes", "description": "Lists the notes.", "enabled": true}])
                        } else {
                            json!([])
                        };
                        json!({"name": name, "displayName": name, "kind": "server",
                            "transport": "streamable-http", "status": status, "tools": tools,
                            "error": null, "pluginName": null})
                    })
                    .collect();
                sources.push(json!({"name": "vw", "displayName": "vw", "kind": "server", "transport": "stdio",
                    "status": "connected", "tools": [{"name": "pages_read", "description": "", "enabled": true}],
                    "error": null, "pluginName": "vibe-work"}));
                for source in session_sources(&w) {
                    sources.push(
                        json!({"name": source["alias"], "displayName": source["displayName"],
                        "kind": "connector", "transport": "connector", "status": source["status"],
                        "tools": source["tools"], "error": null, "pluginName": null}),
                    );
                }
                json!({"mcp": {"sources": sources, "discoveryErrors": {}, "connectorError": null,
                    "manageConnectorsUrl": null}})
            }
            "mcp_catalog/refresh" => json!({"runtime": null}),
            "mcp_catalog/add" => {
                let url = params["url"].as_str().unwrap_or_default().to_owned();
                if let Some((name, _)) = w.servers.iter().find(|(_, s)| s.url == url) {
                    return json!({"name": name, "url": url, "created": false, "runtime": null});
                }
                let name = params["name"].as_str().map_or_else(
                    || {
                        url.split("//")
                            .nth(1)
                            .and_then(|r| r.split([':', '/']).next())
                            .unwrap_or("server")
                            .replace('.', "_")
                    },
                    str::to_owned,
                );
                if w.servers.contains_key(&name) {
                    return error(
                        "invalid_params",
                        &format!("MCP server name `{name}` is already configured."),
                    );
                }
                w.servers.insert(
                    name.clone(),
                    Server {
                        url: url.clone(),
                        ..Server::default()
                    },
                );
                persist(&w);
                json!({"name": name, "url": url, "created": true, "runtime": null})
            }
            "mcp_catalog/logout" => {
                let name = params["name"].as_str().unwrap_or_default();
                match w.servers.get_mut(name) {
                    Some(server) => {
                        server.signed_in = false;
                        json!({"runtime": null})
                    }
                    None => error("invalid_params", "Unknown MCP server"),
                }
            }
            "mcp_catalog/remove" => {
                let name = params["name"].as_str().unwrap_or_default().to_owned();
                let removed = w.servers.remove(&name).is_some();
                persist(&w);
                json!({"name": name, "removed": removed, "runtime": null})
            }
            "mcp_catalog/toggle" => {
                if params["source"].as_str() == Some("connector") {
                    return error(
                        "not_implemented",
                        "Connector-owned MCP sources are not part of the MCP catalog",
                    );
                }
                let name = params["name"].as_str().unwrap_or_default();
                let Some(server) = w.servers.get_mut(name) else {
                    return error("invalid_params", "Unknown MCP server");
                };
                server.disabled = params["disabled"].as_bool() == Some(true);
                persist(&w);
                json!({"runtime": null})
            }
            other => json!({"echo": other}),
        }
    }

    /// `mcp_catalog/login`: publishes the page address, then waits for the browser step (max 60 s).
    /// Runs on its own thread (`main.rs`), like Vibe's request that only answers after the callback.
    pub fn login_blocking(&self, conn: u64, params: &Value) -> Value {
        let f = fake();
        let name = params["name"].as_str().unwrap_or_default().to_owned();
        if !f.world.lock().unwrap().servers.contains_key(&name) {
            return error("invalid_params", &format!("Unknown MCP server: {name}"));
        }
        if name.starts_with("noauth") {
            std::thread::sleep(Duration::from_millis(100));
            return error(
                "invalid_params",
                &format!(
                    "OAuth login failed for MCP server '{name}': the server never issued an OAuth challenge, so no token was obtained. Run `/mcp login {name}` to retry."
                ),
            );
        }
        f.world.lock().unwrap().pending_logins.insert(name.clone());
        let url = format!(
            "http://127.0.0.1:{}/authorize?server={name}&state=fake-state&code_challenge=fake",
            f.port
        );
        if let Some(send) = self.send_to(conn) {
            let payload = json!({"name": name, "url": url});
            send(json!({"jsonrpc": "2.0", "method": "mcp_catalog/authUrl", "params": payload}));
            send(json!({"jsonrpc": "2.0", "method": "mcp/authUrl", "params": payload}));
        }
        let w = f.world.lock().unwrap();
        let (w, timeout) = f
            .done
            .wait_timeout_while(w, Duration::from_secs(60), |w| {
                w.pending_logins.contains(&name)
            })
            .unwrap();
        drop(w);
        if timeout.timed_out() {
            f.world.lock().unwrap().pending_logins.remove(&name);
            return error(
                "invalid_params",
                &format!("OAuth login failed for MCP server '{name}': no callback (fake timeout)"),
            );
        }
        json!({"runtime": null})
    }
}
