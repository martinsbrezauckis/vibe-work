//! The exact allowlist of Mistral endpoints the proxy forwards (Vibe 2.26.0
//! sources and the mistralai SDK paths).
//! Anything else is refused with 403 and never forwarded.
//!
//! Path layout on the proxy (what Vibe is configured with, vw-keys `vibe_env`):
//! - `/v1/...` → the API host (provider `api_base` = `http://127.0.0.1:<port>/v1`; Vibe derives the
//!   connector gateway, telemetry and web-search server from it by dropping `/v1`);
//! - `/console/...` → the console host (`console_base_url`: `account/read` and experiments call
//!   `/api/vibe/whoami` there with the key);
//! - `/chat/...` → the chat host (`vibe_base_url`: the managed admin config).
//!
//! Refused on purpose: transcription (`wss://`, CLI voice only; Vibe Work does speech itself),
//! OTEL export (off by default), browser sign-in (Vibe Work never uses Vibe's sign-in), the
//! built-in `lean` agent's hard-coded provider (it never reaches the proxy; with the placeholder it
//! fails closed at Mistral).
//!
//! Depends on: axum's `Method`. Called by: `server`.

use axum::http::Method;

/// Which Mistral host a route goes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Upstream {
    /// `api.mistral.ai`.
    Api,
    /// `console.mistral.ai`.
    Console,
    /// `chat.mistral.ai`.
    Chat,
}

/// One allowlisted endpoint: method, path template (`{}` = one path segment of letters, digits and
/// `-_.:`), upstream host, and the prefix the proxy strips before forwarding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RouteSpec {
    /// HTTP method.
    pub method: &'static str,
    /// Proxy path template.
    pub template: &'static str,
    /// Upstream host.
    pub upstream: Upstream,
    /// Prefix removed before forwarding (`""`, `/console`, `/chat`).
    pub strip: &'static str,
}

const fn api(method: &'static str, template: &'static str) -> RouteSpec {
    RouteSpec {
        method,
        template,
        upstream: Upstream::Api,
        strip: "",
    }
}

/// The allowlist. Each entry names the Vibe code that calls it.
pub const ALLOWLIST: &[RouteSpec] = &[
    // Model calls (harness Mistral adapter, utility completions: titles, smart approve).
    api("POST", "/v1/chat/completions"),
    // Model list (Vibe's model probe; our own test call goes direct, not through here).
    api("GET", "/v1/models"),
    api("GET", "/v1/models/{}"),
    // RegistrySkillsClient (Vibe 2.26.1, core/skills/registry/_client.py): catalog, pinned payload, versions.
    api("GET", "/v1/skills"),
    api("GET", "/v1/skills/{}"),
    api("GET", "/v1/skills/{}/versions"),
    // Web search tool (`web_search.py`: `beta.conversations.start_async`).
    api("POST", "/v1/conversations"),
    // Identity for experiments (`core/identity.py`: `/users/me` under `api_base`).
    api("GET", "/v1/users/me"),
    // Telemetry events (`telemetry/send.py`; Vibe sends them only when telemetry is on).
    api("POST", "/v1/datalake/events"),
    // Narration (`audio.speech.complete_async`, via the `tts_providers` override).
    api("POST", "/v1/audio/speech"),
    // Connectors: catalog bootstrap, list, auth URL, and the MCP gateway (streamable HTTP).
    api("GET", "/v1/connectors/bootstrap"),
    api("GET", "/v1/connectors"),
    api("GET", "/v1/connectors/{}/auth_url"),
    // Connect of a connector that needs no sign-in (Vibe 3.0.3 `connector_catalog/connect`): the user-scope
    // credential write only, and only with an empty header set (`credentials.rs` checks the body).
    api("PATCH", "/v1/connectors/{}/user/credentials"),
    api("GET", "/v1/connectors-gateway/{}/mcp"),
    api("POST", "/v1/connectors-gateway/{}/mcp"),
    api("DELETE", "/v1/connectors-gateway/{}/mcp"),
    // `account/read` and experiments (`setup/auth/whoami.py:27`).
    RouteSpec {
        method: "GET",
        template: "/console/api/vibe/whoami",
        upstream: Upstream::Console,
        strip: "/console",
    },
    // Managed admin config (`core/config/admin_config.py:15`).
    RouteSpec {
        method: "GET",
        template: "/chat/api/v1/code/managed-config",
        upstream: Upstream::Chat,
        strip: "/chat",
    },
];

fn segment_ok(segment: &str) -> bool {
    !segment.is_empty()
        && segment
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':'))
        && segment != "."
        && segment != ".."
}

fn matches(template: &str, path: &str) -> bool {
    let mut t = template.split('/');
    let mut p = path.split('/');
    loop {
        match (t.next(), p.next()) {
            (None, None) => return true,
            (Some("{}"), Some(seg)) if segment_ok(seg) => {}
            (Some(a), Some(b)) if a == b => {}
            _ => return false,
        }
    }
}

/// The allowlist entry for `method` + `path` (path without query), if any.
#[must_use]
pub fn match_route(method: &Method, path: &str) -> Option<&'static RouteSpec> {
    ALLOWLIST
        .iter()
        .find(|r| r.method == method.as_str() && matches(r.template, path))
}
