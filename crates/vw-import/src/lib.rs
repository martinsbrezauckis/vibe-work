//! Bring your setup from another coding agent: a read-only scan of four agents' setups (see [`Agent`])
//! into a plan of rows (instructions, skills, MCP servers, and what cannot be brought, with why), for the person to
//! review before anything is written.
//!
//! Invariants:
//! - The scan only reads, through [`fs::Fs`], which refuses sign-in and secret files before opening them
//!   ([`fs::never_read`]), follows no link and caps sizes and counts.
//! - A plan row never holds a key's value: only its name ([`Found::keys`]). Values exist only in [`Payload`], which
//!   is not serializable and whose `Debug` names keys without values; the core moves them straight into the vault.
//! - Nothing here writes; the core applies a plan after the person's confirmation.
//!
//! Depends on: serde, `serde_json`, `toml`. Called by: vw-core-api (`import`).

pub mod agents;
pub mod fs;
pub mod mcp;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The agents whose setups are read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(rename = "ImportAgent"))]
#[serde(rename_all = "camelCase")]
pub enum Agent {
    /// The agent configured in `~/.claude/`.
    ClaudeCode,
    /// The Codex command line agent.
    Codex,
    /// OpenCode.
    OpenCode,
    /// Cursor.
    Cursor,
}

impl Agent {
    /// The name shown to the person.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::ClaudeCode => "Claude Code",
            Self::Codex => "Codex",
            Self::OpenCode => "OpenCode",
            Self::Cursor => "Cursor",
        }
    }
}

/// What a row is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(rename = "ImportKind"))]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    /// Personal instructions.
    Instructions,
    /// A skill folder.
    Skill,
    /// An MCP server (connector).
    McpServer,
    /// Permission or approval rules.
    Permissions,
    /// Hooks.
    Hooks,
    /// A subagent definition.
    Subagent,
    /// A custom command or prompt.
    Command,
    /// Project rules.
    Rules,
}

/// Whether a row can be brought.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(rename = "ImportStatus"))]
#[serde(rename_all = "camelCase")]
pub enum Status {
    /// Brought as it is.
    Ready,
    /// Brought with a change (the note says which).
    Changed,
    /// Not brought (the note says why).
    CantBring,
}

/// One row of the plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(rename = "ImportFound"))]
#[serde(rename_all = "camelCase")]
pub struct Found {
    /// Stable id for picking (agent, kind, name and source).
    pub id: String,
    /// Where it comes from.
    pub agent: Agent,
    /// The agent's name as shown.
    pub agent_label: String,
    /// What it is.
    pub kind: Kind,
    /// Its name.
    pub name: String,
    /// The file or folder it was read from.
    pub source: String,
    /// Where it goes in Vibe Work (`None` when it cannot be brought).
    pub target: Option<String>,
    /// Ready, changed or cannot be brought.
    pub status: Status,
    /// The change, or why it cannot be brought.
    pub note: Option<String>,
    /// The exact command a program server runs, or a server's address.
    pub detail: Option<String>,
    /// Names of keys that go to the vault (never their values).
    pub keys: Vec<String>,
    /// Changes when the source changes (apply refuses a row whose source changed since the person saw it).
    pub fingerprint: String,
}

/// What the core needs to apply a row; never serialized, never logged with values.
pub enum Payload {
    /// Instructions text to append under a heading.
    Instructions(String),
    /// A skill folder to copy (its files, relative path and bytes).
    Skill(Vec<(PathBuf, Vec<u8>)>),
    /// An MCP server definition (key values only here).
    Server(mcp::Server),
    /// Nothing to apply.
    None,
}

impl std::fmt::Debug for Payload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Instructions(text) => write!(f, "Instructions({} bytes)", text.len()),
            Self::Skill(files) => write!(f, "Skill({} files)", files.len()),
            Self::Server(server) => write!(f, "Server({server:?})"),
            Self::None => f.write_str("None"),
        }
    }
}

/// Where to look.
#[derive(Debug, Clone)]
pub struct Homes {
    /// The person's home folder.
    pub home: PathBuf,
    /// `$CODEX_HOME` when set (else `~/.codex`).
    pub codex_home: Option<PathBuf>,
    /// The per-user config folder OpenCode uses: `$XDG_CONFIG_HOME` or `~/.config`, on every system (OpenCode
    /// does not use `%APPDATA%` or `~/Library`).
    pub config_dir: PathBuf,
    /// A project the person picked (its own files), if any.
    pub project: Option<PathBuf>,
}

/// Names already used in Vibe Work's own setup (a row with the same name cannot be brought).
#[derive(Debug, Clone, Default)]
pub struct Taken {
    /// Skill folder names in the managed skills folder.
    pub skills: Vec<String>,
    /// MCP server names in the managed config.
    pub servers: Vec<String>,
}

/// The agents with a personal setup worth offering, by existence checks only (for the first-start hint).
#[must_use]
pub fn detect(fs: &dyn fs::Fs, homes: &Homes) -> Vec<Agent> {
    agents::detect(fs, homes)
}

/// The plan rows only (what the window shows).
#[must_use]
pub fn scan(fs: &dyn fs::Fs, homes: &Homes, taken: &Taken) -> Vec<Found> {
    scan_with_payloads(fs, homes, taken)
        .into_iter()
        .map(|(found, _)| found)
        .collect()
}

/// The plan rows with what applying each needs (the core's apply; payloads never leave the core).
#[must_use]
pub fn scan_with_payloads(fs: &dyn fs::Fs, homes: &Homes, taken: &Taken) -> Vec<(Found, Payload)> {
    let mut rows = agents::scan_all(fs, homes);
    for (found, _) in &mut rows {
        let clash = match found.kind {
            Kind::Skill => taken.skills.contains(&found.name),
            Kind::McpServer => taken.servers.contains(&found.name),
            _ => false,
        };
        if clash && found.status != Status::CantBring {
            found.status = Status::CantBring;
            found.target = None;
            found.note = Some(format!(
                "Vibe Work already has a {} with this name.",
                if found.kind == Kind::Skill {
                    "skill"
                } else {
                    "server"
                }
            ));
        }
    }
    rows
}

/// A short, stable digest of bytes (change detection, not security).
#[must_use]
pub fn fingerprint(parts: &[&[u8]]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for part in parts {
        for byte in *part {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
        hash ^= 0xff;
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

#[cfg(test)]
mod credential_tests;
