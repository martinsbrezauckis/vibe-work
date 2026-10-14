//! Credentials in Vibe's git listings (replacing the `debug_redact`).
//!
//! `workspace/git/checkouts` answers with each checkout's remote address (`repoUrl`) and Vibe's error text
//! (`message`, which repeats git's output); both can carry `user:token@` in an address (a remote configured
//! with a token). Under the Debug API rule (everything as the user,
//! never a stored secret as a value) paths and names are returned to every client, but the user info of every
//! address in these answers is removed, for the window and the Debug API alike (no screen needs it).
//!
//! Depends on: `serde_json`. Called by: `dispatch` (`forward`, after the gate's reply).
//! Invariants:
//! - Only `workspace/git/checkouts` and `workspace/git/worktrees/list` are touched; every other reply passes
//!   through unchanged.
//! - Every string anywhere in those answers is scanned (fail closed for fields Vibe adds later): each
//!   `<scheme>://<userinfo>@` loses its `<userinfo>@`, and nothing else changes.

use serde_json::Value;

/// Methods whose answers can hold remote addresses.
const GIT_LISTINGS: &[&str] = &["workspace/git/checkouts", "workspace/git/worktrees/list"];

/// The reply `result` of `method` with credentials removed from every address (module docs).
#[must_use]
pub(crate) fn git_listing(method: &str, mut result: Value) -> Value {
    if GIT_LISTINGS.contains(&method) {
        scrub_value(&mut result);
    }
    result
}

fn scrub_value(value: &mut Value) {
    match value {
        Value::String(text) => {
            if text.contains("://") {
                *text = without_user_info(text);
            }
        }
        Value::Array(items) => items.iter_mut().for_each(scrub_value),
        Value::Object(object) => object.values_mut().for_each(scrub_value),
        _ => {}
    }
}

/// `text` with the user info removed from every `scheme://userinfo@host` in it. The user info ends at the last
/// `@` before the next `/`, `?`, `#` or whitespace (a password may contain `@`, a host never does).
#[must_use]
pub(crate) fn without_user_info(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("://") {
        let (head, tail) = rest.split_at(at + 3);
        out.push_str(head);
        let authority_end = tail
            .find(|c: char| matches!(c, '/' | '?' | '#') || c.is_whitespace())
            .unwrap_or(tail.len());
        let authority = &tail[..authority_end];
        match authority.rfind('@') {
            Some(user_end) => out.push_str(&authority[user_end + 1..]),
            None => out.push_str(authority),
        }
        rest = &tail[authority_end..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn user_info_is_removed_from_every_address() {
        assert_eq!(
            without_user_info("https://bob:t0k@n@example.com/r.git and ssh://git@host:22/x"),
            "https://example.com/r.git and ssh://host:22/x"
        );
        assert_eq!(without_user_info("no address here"), "no address here");
        assert_eq!(
            without_user_info("https://example.com/a@b"),
            "https://example.com/a@b"
        );
    }

    #[test]
    fn only_git_listings_are_touched_and_paths_stay() {
        let answer = json!({"checkouts": [{"root": "/home/u/r", "repoUrl": "https://u:secret@h/r.git",
            "message": "fatal: https://u:secret@h/r.git not found"}]});
        let out = git_listing("workspace/git/checkouts", answer.clone());
        assert_eq!(out["checkouts"][0]["root"], "/home/u/r");
        assert_eq!(out["checkouts"][0]["repoUrl"], "https://h/r.git");
        assert!(!out.to_string().contains("secret"));
        assert_eq!(git_listing("session/read", answer.clone()), answer);
    }
}
