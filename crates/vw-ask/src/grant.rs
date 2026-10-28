//! Session grants Vibe Work keeps for Vibe's approvals. Pure: the core extracts the
//! permissions from Vibe's callback and stores grants; this module only says what a grant is and when it covers
//! a later call. Depends on serde.
//!
//! Match rule: equality. A later call is covered when its tool is the grant's tool and its set of derived
//! permissions equals the grant's set (order-free). Equality can only be narrower than Vibe's own glob matching:
//! a call Vibe would match but whose derived patterns differ is asked again, never the reverse.
//!
//! Why `exact` is part of a permission: Vibe matches some patterns as text, never as globs (`RequiredPermission
//! .literal`, `vibe/permissions.py:47`), and that flag is excluded from the wire. A literal pattern is always the
//! command's own text (`invocation_pattern` == `session_pattern` up to whitespace, `bash.py:364-366,605-607`),
//! while a glob is derived from it (`git log *` from `git log --oneline`). Comparing the text alone would let a
//! grant earned by the literal command `git log *` cover the glob `git log *` of `git log --ext-diff`, wider
//! than Vibe. Keeping `exact` (pattern equals the invocation) in the key keeps the two apart.
//!
//! What a grant covers is also said in words ([`CoversPart`]): the core builds them from the
//! same keys the matcher uses, so the card ("Allow for this session covers …") and the session's grant list cannot
//! drift from what is matched. Those words hold Vibe's own patterns (the text the card shows), so they live on the
//! grant and on the grant offer of an open card, never on the value-free `Ask` record.
use std::collections::BTreeSet;

use crate::AnsweredBy;

/// One required permission as a grant keeps it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct GrantPermission {
    /// Vibe's `PermissionScope` (`command_pattern`, `outside_directory`, `file_pattern`, `url_pattern`).
    pub scope: String,
    /// Vibe's `sessionPattern`, as sent.
    pub session_pattern: String,
    /// The pattern is the call's own text (Vibe may match it literally); see the module docs.
    pub exact: bool,
}

impl GrantPermission {
    /// The grant form of one wire permission. `exact` is true when the session pattern is the invocation's own
    /// text with runs of whitespace collapsed (Vibe builds literal scopes as `" ".join(tokens)` from Python's
    /// `str.split()`, which also splits on `\x1c`..`\x1f`; [`python_split_joined`]).
    #[must_use]
    pub fn from_wire(scope: &str, invocation_pattern: &str, session_pattern: &str) -> Self {
        let collapsed = python_split_joined(invocation_pattern);
        Self {
            scope: scope.to_owned(),
            session_pattern: session_pattern.to_owned(),
            exact: session_pattern == invocation_pattern || session_pattern == collapsed,
        }
    }
}

/// `" ".join(text.split())` as Python computes it: Python's whitespace is Rust's `char::is_whitespace` plus the
/// separators `\x1c`..`\x1f`. Splitting on fewer characters than Vibe would read a literal scope as a glob.
#[must_use]
pub fn python_split_joined(text: &str) -> String {
    text.split(|c: char| c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c))
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// One piece of a "covers" sentence: plain words, or text shown as code (a pattern, a command, a path, a tool
/// name). The pieces joined are the sentence; nothing is parsed out of the text, so a pattern may hold any character.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct CoversPart {
    /// The text.
    pub text: String,
    /// Shown as code (monospace).
    pub code: bool,
}

impl CoversPart {
    /// Plain words.
    #[must_use]
    pub fn words(text: &str) -> Self {
        Self {
            text: text.to_owned(),
            code: false,
        }
    }
    /// Text shown as code.
    #[must_use]
    pub fn code(text: &str) -> Self {
        Self {
            text: text.to_owned(),
            code: true,
        }
    }
}

/// What "Allow for this session" on an open card would keep, in words (the card's covers line). One per open
/// approval that offers the session answer; gone when the card is answered or closes.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct GrantOffer {
    /// Vibe's callback id of the open card.
    pub callback_id: String,
    /// The session of the card.
    pub session: String,
    /// What the grant would cover, first word in lower case ("commands that match `npm test *`").
    pub covers: Vec<CoversPart>,
}

/// "Allow for this session" kept by Vibe Work: in memory, ends with the session.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SessionGrant {
    /// Run-local identity (Remove names it).
    pub id: String,
    /// The session whose calls it answers.
    pub session: String,
    /// Vibe's tool name (`effect.toolName`).
    pub tool: String,
    /// The permissions it covers, sorted; empty for an every-call grant.
    pub permissions: Vec<GrantPermission>,
    /// Vibe's labels for the permissions, for display only (never used to match).
    pub labels: Vec<String>,
    /// Covers every call of `tool` in the session (the request carried no required permissions).
    pub every_call: bool,
    /// Unix milliseconds.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub created_at: u64,
    /// Who chose "Allow for this session".
    pub by: AnsweredBy,
    /// What it covers, in words (the card's covers line; first word in lower case).
    #[serde(default)]
    pub covers: Vec<CoversPart>,
    /// Calls it answered since it was kept (this run; grants are never stored).
    #[serde(default)]
    pub uses: u32,
    /// When it last answered a call, Unix milliseconds; `None` before its first use.
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub last_used_at: Option<u64>,
}

impl SessionGrant {
    /// Whether this grant answers a call of `tool` in `session` that requires exactly `permissions`.
    #[must_use]
    pub fn covers(&self, session: &str, tool: &str, permissions: &[GrantPermission]) -> bool {
        if self.session != session || self.tool != tool {
            return false;
        }
        let wanted: BTreeSet<&GrantPermission> = permissions.iter().collect();
        let held: BTreeSet<&GrantPermission> = self.permissions.iter().collect();
        if self.every_call {
            // Same as Vibe's own session grant for a tool without permissions: the whole tool. A call that does
            // carry permissions is a different question and is asked again.
            return wanted.is_empty();
        }
        !held.is_empty() && wanted == held
    }
}
