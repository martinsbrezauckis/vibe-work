//! What counts as a secret in a command-started MCP server's arguments and environment values.
//! v1 saves these values in Vibe's config in plain text, so a value that looks like a key is refused before anything is
//! shown or saved (the design for vault-backed values is a later change; the stored form of plain values stays).
//!
//! Depends on: `vw_secrets` (values the app already knows as secrets), `vw_git::checklog` (token prefixes, private-key
//! lines), `zxcvbn` (random-looking strings). Called by: `command_servers` (refusal), `connectors_config` (hidden values).
//! Invariants: a verdict never quotes the value; false alarms are cheap (the refusal names the way forward), a missed key
//! is not, so the rules lean towards refusing.

/// Names that carry a credential, compared in lower case.
const SECRET_NAMES: &[&str] = &[
    "token",
    "secret",
    "password",
    "passwd",
    "api_key",
    "apikey",
    "api-key",
    "credential",
    "private_key",
    "privatekey",
];

/// What the Tools page shows instead of an argument that looks like a key (an older entry, saved before this rule).
pub(crate) const HIDDEN_ARGUMENT: &str = "[hidden]";

/// Longest value the random-looking check reads (the check is slow on long text; a longer value is not a key).
const MAX_ENTROPY_CHECK: usize = 256;

/// True when `value` (of the variable `name`, or of an argument when `None`) looks like a key, token or password.
#[must_use]
pub(crate) fn looks_like_secret(name: Option<&str>, value: &str) -> bool {
    if value.is_empty() {
        return false;
    }
    // A value of the vault or another stored secret; the credentials of other servers are not counted (every
    // environment value in the config is registered as one, so plain values would refuse each other).
    if vw_secrets::find(value.as_bytes())
        .is_some_and(|kind| kind != vw_secrets::SecretKind::ConnectorCredential)
    {
        return true;
    }
    if vw_git::checklog::scrub_line(value) != value {
        return true;
    }
    if let Some(name) = name {
        let lower = name.to_ascii_lowercase();
        if value.chars().count() >= 8 && SECRET_NAMES.iter().any(|n| lower.contains(n)) {
            return true;
        }
    }
    let plain_text = value.chars().any(char::is_whitespace)
        || value.contains(['/', '\\', ':'])
        || value.chars().count() < 20
        || value.chars().count() > MAX_ENTROPY_CHECK;
    !plain_text && zxcvbn::zxcvbn(value, &[]).guesses_log10() >= 16.0
}

#[cfg(test)]
mod tests {
    use super::looks_like_secret;

    #[test]
    fn keys_are_recognised_and_ordinary_settings_are_not() {
        for secret in [
            "sk-proj-abc1234567890abcdef1234",
            "ghp_1234567890abcdefghijklmnopqrstuvwxyz",
            "-----BEGIN PRIVATE KEY-----",
        ] {
            assert!(looks_like_secret(None, secret), "{secret}");
        }
        assert!(looks_like_secret(Some("API_TOKEN"), "hunter2hunter2"));
        assert!(looks_like_secret(None, "Zq8xV1mK4pR7tW2yB5nC9dF3gH6jL0sA"));
        for plain in [
            "info",
            "on",
            "/home/u/data",
            "https://example.org/x",
            "@scope/pkg@1.2.3",
            "--mode",
        ] {
            assert!(!looks_like_secret(None, plain), "{plain}");
        }
        assert!(!looks_like_secret(Some("MCP_LOG"), "debug"));
        assert!(!looks_like_secret(Some("TOKENIZERS_PARALLELISM"), "false"));
    }
}
