//! Billing allowlist boundary tests. Uses the public matcher; cargo tests both feature states.
//! Only two exact GETs may be added; every existing route and refusal must stay intact.

use axum::http::Method;
use vw_keyproxy::{ALLOWLIST, Upstream, match_route};

#[test]
fn billing_reads_require_the_optional_feature() {
    for path in ["/v1/billing/subscription", "/v1/billing/budget"] {
        let route = match_route(&Method::GET, path);
        assert_eq!(route.is_some(), cfg!(feature = "plan-usage"), "{path}");
        if let Some(route) = route {
            assert_eq!(route.upstream, Upstream::Api);
            assert_eq!(route.template, path);
            assert_eq!(route.strip, "");
        }
    }
}

#[test]
fn billing_never_allows_writes_other_paths_or_encoded_aliases() {
    for path in ["/v1/billing/subscription", "/v1/billing/budget"] {
        for method in [
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
            Method::HEAD,
            Method::OPTIONS,
            Method::TRACE,
            Method::CONNECT,
        ] {
            assert!(match_route(&method, path).is_none(), "{method} {path}");
        }
        for suffix in ["/", "/extra", ".json", "%2f", "/../budget"] {
            let alias = format!("{path}{suffix}");
            assert!(match_route(&Method::GET, &alias).is_none(), "{alias}");
        }
    }
    for path in [
        "/v1/billing",
        "/v1/billing/invoices",
        "/v1/billing/usage",
        "/v1/billing/Budget",
        "/v1/billing/%62udget",
        "/v1/billing//budget",
        "/v1/billing/./budget",
        "/v1/billing/../budget",
        "/v1/billing/budget?other=1",
        "/console/v1/billing/budget",
        "/chat/v1/billing/budget",
    ] {
        assert!(match_route(&Method::GET, path).is_none(), "{path}");
    }
}

#[test]
fn the_rest_of_the_allowlist_is_unchanged() {
    let actual: Vec<_> = ALLOWLIST
        .iter()
        .filter(|route| !route.template.starts_with("/v1/billing/"))
        .map(|route| (route.method, route.template, route.upstream, route.strip))
        .collect();
    let api = Upstream::Api;
    assert_eq!(
        actual,
        vec![
            ("POST", "/v1/chat/completions", api, ""),
            ("GET", "/v1/models", api, ""),
            ("GET", "/v1/models/{}", api, ""),
            ("GET", "/v1/skills", api, ""),
            ("GET", "/v1/skills/{}", api, ""),
            ("GET", "/v1/skills/{}/versions", api, ""),
            ("POST", "/v1/conversations", api, ""),
            ("GET", "/v1/users/me", api, ""),
            ("POST", "/v1/datalake/events", api, ""),
            ("POST", "/v1/audio/speech", api, ""),
            ("GET", "/v1/connectors/bootstrap", api, ""),
            ("GET", "/v1/connectors", api, ""),
            ("GET", "/v1/connectors/{}/auth_url", api, ""),
            ("PATCH", "/v1/connectors/{}/user/credentials", api, ""),
            ("GET", "/v1/connectors-gateway/{}/mcp", api, ""),
            ("POST", "/v1/connectors-gateway/{}/mcp", api, ""),
            ("DELETE", "/v1/connectors-gateway/{}/mcp", api, ""),
            (
                "GET",
                "/console/api/vibe/whoami",
                Upstream::Console,
                "/console"
            ),
            (
                "GET",
                "/chat/api/v1/code/managed-config",
                Upstream::Chat,
                "/chat"
            ),
        ]
    );
    assert_eq!(
        ALLOWLIST.len(),
        actual.len() + if cfg!(feature = "plan-usage") { 2 } else { 0 }
    );
}
