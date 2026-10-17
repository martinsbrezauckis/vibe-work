//! The value rule of `config/proxy/write` (Vibe's `/proxy-setup`), and the typed check that applies it.
//!
//! Depends on: vw-mistral (`network`: the same checks the Settings screen runs), `table::typed`. Called by: the
//! method table (the `config/proxy/write` row), so a refused value never reaches the native dialog or Vibe.
//! Invariants:
//! - Only Vibe's six keys, spelled as Vibe lists them (Vibe upper-cases others, but a dialog line must name the
//!   key it will really write), and at least one of them.
//! - A value is `null` or `""` (remove the key) or text in its canonical form: a proxy address is `http://` or
//!   `https://` with a host and an optional port, a no-proxy list is comma-joined entries without spaces, a
//!   certificate path is absolute. The gate never rewrites params, so the core sends the canonical form.
//! - Characters Vibe's `.env` writer would damage are refused (control characters, `${`, a doubled backslash).
//! - A refusal is a fixed sentence about the setting: it never quotes the value (a proxy address can carry
//!   `user:password@`), and an unknown key name is not quoted either.
//! - Whether a certificate file exists is not decided here (no I/O in the gate); the core checks that.

use serde_json::Value;
use vw_mistral::network::{KEYS, check_no_proxy, check_path_text, check_proxy_url};

/// The table check of `config/proxy/write`: the generated type (via `typed`), then the value rules.
pub(crate) fn check_write<M>(params: &Value) -> Result<(), String>
where
    M: vw_protocol::RequestMethod,
    M::Params: serde::de::DeserializeOwned + serde::Serialize,
{
    crate::table::typed::<M>(params)?;
    let changes = params
        .get("changes")
        .and_then(Value::as_object)
        .ok_or_else(|| "changes must be an object".to_owned())?;
    if changes.is_empty() {
        return Err("changes must name at least one setting".to_owned());
    }
    for (key, value) in changes {
        let Some(key) = KEYS.iter().find(|known| **known == key.as_str()) else {
            return Err(format!(
                "a setting name is not one of Vibe's proxy settings ({})",
                KEYS.join(", ")
            ));
        };
        let text = match value {
            Value::Null => continue,
            Value::String(text) if text.is_empty() => continue,
            Value::String(text) => text,
            _ => return Err(format!("{key} must be text or null")),
        };
        let checked = match *key {
            "HTTP_PROXY" | "HTTPS_PROXY" | "ALL_PROXY" => check_proxy_url(key, text),
            "NO_PROXY" => check_no_proxy(key, text),
            _ => check_path_text(key, text),
        }?;
        if checked != *text {
            return Err(format!(
                "{key} must be given trimmed and in its plain form (lower-case scheme, entries joined by commas)"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use vw_protocol::request::ConfigProxyWrite;

    use super::check_write;

    fn check(changes: &serde_json::Value) -> Result<(), String> {
        check_write::<ConfigProxyWrite>(&json!({"sessionId": "s", "changes": changes}))
    }

    /// An absolute certificate path on this computer: Windows needs a drive, so `/etc/...` is not absolute there.
    const CERT_FILE: &str = if cfg!(windows) {
        "C:\\certs\\company-ca.pem"
    } else {
        "/etc/ssl/company-ca.pem"
    };

    #[test]
    fn accepts_each_kind_of_value_and_removal() {
        assert_eq!(
            check(&json!({
                "HTTP_PROXY": "http://proxy.corp:8080",
                "HTTPS_PROXY": "https://u:p@proxy.corp:8443",
                "ALL_PROXY": null,
                "NO_PROXY": ".corp,10.0.0.0/8,localhost",
                "SSL_CERT_FILE": CERT_FILE,
                "SSL_CERT_DIR": ""
            })),
            Ok(())
        );
    }

    #[test]
    fn refuses_what_vibes_writer_would_damage_or_vibe_would_not_start_with() {
        for changes in [
            json!({}),
            json!({"FOO": "x"}),
            json!({"https_proxy": "http://p:1"}),
            json!({"HTTP_PROXY": "proxy.corp:8080"}),
            json!({"HTTP_PROXY": "socks5://p:1080"}),
            json!({"HTTP_PROXY": " http://p:1"}),
            json!({"HTTP_PROXY": "HTTP://p:1"}),
            json!({"HTTP_PROXY": "http://u:${HOME}@p:1"}),
            json!({"NO_PROXY": "a, b"}),
            json!({"SSL_CERT_FILE": "ca.pem"}),
            json!({"SSL_CERT_FILE": "\\\\server\\share\\ca.pem"}),
            json!({"SSL_CERT_DIR": "/certs\n/other"}),
            json!({"HTTP_PROXY": 5}),
            json!({"HTTPS_PROXY": "http://evil.example:3128#@proxy.corp:8080"}),
            json!({"HTTPS_PROXY": "http://evil.example:3128?@proxy.corp:8080"}),
            json!({"HTTPS_PROXY": "http://evil.example:3128/@proxy.corp:8080"}),
            json!({"HTTPS_PROXY": "http://evil.example:3128\\@proxy.corp:8080"}),
        ] {
            assert!(check(&changes).is_err(), "{changes}");
        }
    }

    #[test]
    fn a_refusal_never_quotes_the_value_or_the_unknown_key() {
        let hostile = check(&json!({"secret-key-name": "x"})).expect_err("unknown key");
        assert!(!hostile.contains("secret-key-name"), "{hostile}");
        let value =
            check(&json!({"HTTPS_PROXY": "hunter2:secret@proxy:8080"})).expect_err("no scheme");
        assert!(
            !value.contains("hunter2") && !value.contains("secret@"),
            "{value}"
        );
    }
}
