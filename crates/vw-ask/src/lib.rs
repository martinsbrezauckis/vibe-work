//! Permission policy and value-free records. Depends on serde; the gate, core and clients share these rules.
//! Presentation and clocks belong to the core; this crate never reads parameters or secret values.
pub mod chrome;
mod grant;
mod policy;
mod text;
pub use chrome::ChromeSettings;
pub use grant::{CoversPart, GrantOffer, GrantPermission, SessionGrant};
pub use policy::{Policy, always_settings, policies, policy};
pub use text::{DECLINED, agent_text, is_hidden_char, plain_note};

/// Who supplied the answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum AnsweredBy {
    /// Person using the native dialog or window.
    Person,
    /// Debug API acting as the user outside the vault.
    DebugApi,
    /// Core finishing an earlier user choice, or refusing a yes on its own (a removed connector's tool).
    Core,
    /// Vibe settled its own callback.
    Vibe,
    /// A session grant Vibe Work keeps ("Allow for this session" earlier in the same session) answered it.
    Grant,
}
/// The permission's lifetime. Existing lasting choices are retained during migration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum Scope {
    /// This exact call.
    Once,
    /// This session.
    Session,
    /// An existing lasting grant.
    Level,
}
/// Where a person answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum Surface {
    /// OS dialog.
    Native,
    /// Chat card.
    Card,
}
/// Terminal answer; one outcome wins for an ask.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "type", rename_all = "camelCase")]
#[cfg_attr(feature = "ts", ts(rename = "AskOutcome"))]
pub enum Outcome {
    /// An answer was attempted, but Vibe did not confirm it. The proposed decision is deliberately absent.
    AnswerNotConfirmed {
        /// Who attempted the answer (not evidence that Vibe received it).
        by: AnsweredBy,
    },
    /// A permission was granted.
    Allowed {
        /// Permission lifetime.
        scope: Scope,
        /// Answer source.
        by: AnsweredBy,
    },
    /// An explicit No.
    Refused {
        /// Answer source.
        by: AnsweredBy,
        /// Plain, scrubbed feedback.
        note: Option<String>,
    },
    /// The caller or session stopped waiting.
    Cancelled {
        /// Plain reason, supplied by the core.
        reason: String,
    },
    /// A preference refuses this class of action.
    Never,
}
/// A Debug API answer, retaining the card's currently offered scopes.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Answer {
    /// once, session or no.
    pub answer: Choice,
    /// Optional plain refusal note.
    pub note: Option<String>,
}
/// Answer names used by the Debug API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum Choice {
    /// Allow this call.
    Once,
    /// Allow in this session when offered.
    Session,
    /// Refuse.
    No,
}
/// Who started a permission request, independently of who eventually answers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum Requester {
    /// A session's agent.
    Agent,
    /// A person using the window.
    Window,
    /// Debug API acting as the user outside the vault.
    DebugApi,
    /// Core completing an earlier choice.
    Core,
}
impl From<AnsweredBy> for Requester {
    fn from(by: AnsweredBy) -> Self {
        match by {
            AnsweredBy::Person => Self::Window,
            AnsweredBy::DebugApi => Self::DebugApi,
            AnsweredBy::Core | AnsweredBy::Vibe | AnsweredBy::Grant => Self::Core,
        }
    }
}
/// Safe metadata for an open ask. Commands, page contents and vault values never enter this type.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Ask {
    /// Run-local identity.
    pub id: String,
    /// Policy method or callback kind.
    pub kind: String,
    /// Owning session when present.
    pub session: Option<String>,
    /// Request source.
    pub requester: Requester,
    /// Presentation surface.
    pub surface: Surface,
    /// Fixed plain policy label, never request contents.
    pub title: String,
    /// Unix milliseconds.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub opened_at: u64,
    /// Allowed wire answers.
    pub offers: Vec<Choice>,
    /// Only the native presenter can answer.
    pub person_only: bool,
    /// "Allow for this session" covers every call of this tool in the session, not one scope: the request
    /// carried no required permissions (MCP and provided tools), so the card must say so before the click.
    pub every_call: bool,
    /// The Settings control (its Settings id) an "Always" answer sets. `None`: the card offers no "Always".
    /// Vibe's own `approve_permanently` is never offered.
    pub always_setting: Option<String>,
}
/// Exactly one terminal record for each ask, retained in memory.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct AskRecord {
    /// Original value-free metadata.
    pub ask: Ask,
    /// Unix milliseconds.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub closed_at: u64,
    /// Winning outcome.
    pub outcome: Outcome,
}

#[cfg(test)]
mod tests;
