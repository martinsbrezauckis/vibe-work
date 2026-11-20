//! The proxy token an inbound request carries: one reading of `Authorization: Bearer <token>` for the model routes
//! (`server_forward.rs`) and the MCP forwarder (`mcp.rs`), so both admit the same header shapes.
//!
//! Depends on: axum's header types. Called by: the model-route admission and the MCP forwarder.

use axum::http::HeaderMap;
use axum::http::header::AUTHORIZATION;

/// The token of the request's first `Authorization` header when it reads `Bearer <token>` (the scheme word exactly
/// as written, one space; spaces around the token are trimmed). `None`: no such header, a value that is not text, or
/// another scheme. A second `Authorization` header is not read; the first one decides.
pub(crate) fn token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::trim)
}

#[cfg(test)]
mod tests {
    use super::token;
    use axum::http::header::AUTHORIZATION;
    use axum::http::{HeaderMap, HeaderValue};

    fn headers(values: &[&[u8]]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for value in values {
            headers.append(AUTHORIZATION, HeaderValue::from_bytes(value).unwrap());
        }
        headers
    }

    #[test]
    fn reads_the_token_of_a_bearer_header() {
        assert_eq!(token(&headers(&[b"Bearer abc"])), Some("abc"));
        assert_eq!(token(&headers(&[b"Bearer   abc  "])), Some("abc"), "spaced");
    }

    #[test]
    fn no_token_without_a_bearer_header() {
        assert_eq!(token(&HeaderMap::new()), None, "absent");
        assert_eq!(token(&headers(&[b"Basic abc"])), None, "another scheme");
        assert_eq!(
            token(&headers(&[b"bearer abc"])),
            None,
            "scheme spelled otherwise"
        );
        assert_eq!(token(&headers(&[b"Bearerabc"])), None, "no space");
        assert_eq!(token(&headers(&[b"Bearer \xff"])), None, "not text");
    }

    #[test]
    fn the_first_of_two_headers_decides() {
        assert_eq!(
            token(&headers(&[b"Bearer one", b"Bearer two"])),
            Some("one")
        );
        assert_eq!(token(&headers(&[b"Basic one", b"Bearer two"])), None);
    }

    /// Both callers read the token through [`token`]: the MCP forwarder's check accepts and refuses the same shapes.
    #[test]
    fn the_mcp_forwarder_accepts_the_same_headers() {
        let accepts = |t: &str| t == "abc";
        assert!(crate::mcp::token_ok(&headers(&[b"Bearer  abc "]), &accepts));
        assert!(!crate::mcp::token_ok(&HeaderMap::new(), &accepts));
        assert!(!crate::mcp::token_ok(&headers(&[b"Basic abc"]), &accepts));
        assert!(crate::mcp::token_ok(
            &headers(&[b"Bearer abc", b"Bearer x"]),
            &accepts
        ));
        assert!(!crate::mcp::token_ok(
            &headers(&[b"Bearer x", b"Bearer abc"]),
            &accepts
        ));
    }
}
