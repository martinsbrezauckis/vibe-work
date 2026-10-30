//! Tools skills command shapes and the display projection of vw-skills responses.
//! Called by dispatch and the TS generator; registry identities and scopes come only from Vibe.
use serde::{Deserialize, Serialize};

/// A skills operation in the selected work folder.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SkillsRequest {
    /// The selected chat, whose working folder is authoritative.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub session_id: Option<String>,
    /// The natively selected new-chat folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub cwd: Option<String>,
    /// The action; all targets are re-resolved against Vibe before a change.
    #[serde(flatten)]
    pub action: SkillsAction,
}

/// The Tools skills operations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "op",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum SkillsAction {
    /// Read installed skills and the catalog without marking updates seen.
    Read {},
    /// Toggle a skill in the selected runtime and its persisted configuration.
    SetEnabled {
        /// Vibe's skill name.
        name: String,
        /// Desired state.
        enabled: bool,
    },
    /// Install the catalog identity globally at latest. With `offer_id` from the window: the click confirms
    /// that shown offer and no native dialog opens; without it the native dialog asks as before.
    Import {
        /// Vibe's registry id.
        skill_id: String,
        /// The id of the `offerImport` offer the window showed.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        offer_id: Option<String>,
    },
    /// Update the active, unambiguous registry copy; `offer_id` as for `import` (from `offerUpdate`).
    SetLatest {
        /// Vibe's skill name.
        name: String,
        /// The id of the `offerUpdate` offer the window showed.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        offer_id: Option<String>,
    },
    /// What Install would add (name, source, publisher, licence, what it adds), as a single-use offer for `import`.
    OfferImport {
        /// Vibe's registry id.
        skill_id: String,
    },
    /// What Update would install, as a single-use offer for `setLatest`.
    OfferUpdate {
        /// Vibe's skill name.
        name: String,
    },
    /// Remove the active registry pin.
    Remove {
        /// Vibe's skill name.
        name: String,
    },
    /// Read the full description, body and registry versions.
    Detail {
        /// Vibe's skill name.
        name: String,
    },
}

/// A skills command response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum SkillsResult {
    /// The authoritative state after a read or mutation.
    View {
        /// Skills and folder activity.
        view: SkillsView,
    },
    /// Details opened under one row.
    Detail {
        /// Full description and content.
        detail: SkillDetail,
    },
    /// `offerImport` / `offerUpdate`: what the click would install; the click sends `offerId` back.
    Offer {
        /// The single-use offer.
        offer: crate::install_offers::ToolsInstallOffer,
    },
}

/// The manager view, separate from any running chat's pinned runtime.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SkillsView {
    /// Rows in Vibe's merged name order.
    pub skills: Vec<SkillRow>,
    /// Configuration issues are session-wide, never guessed onto a row.
    pub issues: Vec<SkillIssue>,
    /// Catalog read completed successfully.
    pub catalog_loaded: bool,
    /// Vibe reports a Mistral registry endpoint.
    pub authenticated: bool,
    /// A chat in this folder is running or waiting for approval.
    pub busy: bool,
}

/// A session-wide configuration problem.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SkillIssue {
    /// Vibe's file name.
    pub file: String,
    /// Vibe's reason.
    pub message: String,
}

/// One row with actions resolved to the active or sole installed copy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SkillRow {
    /// Vibe's name.
    pub name: String,
    /// Unclipped description.
    pub description: String,
    /// Display origin: builtin, user, project, plugin, catalog; absent if ambiguous.
    pub source: Option<String>,
    /// Installed, including ambiguous copies.
    pub installed: bool,
    /// Current state, absent when ambiguous or catalog-only.
    pub enabled: Option<bool>,
    /// Vibe locks this name or the installed copies cannot be resolved safely.
    pub locked: bool,
    /// Plugin/config explanation, absent for an unlocked skill.
    pub lock_reason: Option<String>,
    /// Exact active/sole copy's scope.
    pub scope: Option<String>,
    /// Exact active/sole registry identity, or sole catalog id.
    pub skill_id: Option<String>,
    /// Materialized version.
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub installed_version: Option<i64>,
    /// Latest version for this registry identity.
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub latest_version: Option<i64>,
    /// Only the catalog's `check_updates` signal enables an update button.
    pub update_available: bool,
    /// When the skill file of the active or sole installed copy last changed, in milliseconds since the epoch;
    /// absent for built-in, plugin and catalog-only rows and when the file cannot be found.
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub modified_at: Option<i64>,
}

/// Expanded row content; local descriptions and prompts come from installed/list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SkillDetail {
    /// Skill name.
    pub name: String,
    /// Full description.
    pub description: String,
    /// Full prompt or registry body, if available.
    pub body: Option<String>,
    /// Registry versions as Vibe returns them.
    pub versions: Vec<String>,
    /// Every installed copy's source and scope (Vibe does not return file paths).
    pub locations: Vec<String>,
}

// Flattening and serde's deny_unknown_fields cannot be combined. Decode context first, then let
// the tagged action reject every remaining unknown field rather than silently ignoring it.
impl<'de> Deserialize<'de> for SkillsRequest {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let mut object = serde_json::Map::<String, serde_json::Value>::deserialize(deserializer)?;
        let session_id = object
            .remove("sessionId")
            .map(serde_json::from_value)
            .transpose()
            .map_err(D::Error::custom)?
            .flatten();
        let cwd = object
            .remove("cwd")
            .map(serde_json::from_value)
            .transpose()
            .map_err(D::Error::custom)?
            .flatten();
        let action =
            serde_json::from_value(serde_json::Value::Object(object)).map_err(D::Error::custom)?;
        Ok(Self {
            session_id,
            cwd,
            action,
        })
    }
}
