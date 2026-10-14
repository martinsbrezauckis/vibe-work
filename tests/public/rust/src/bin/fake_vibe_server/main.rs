//! Scripted stand-in for `vibe-app-server`, used by synthetic supervisor tests. Portable (std
//!
//! Protocol: NDJSON JSON-RPC on stdin/stdout like the real server. With `--multiplex` (the supervisor's
//! default mode since the fixture) every line is a channel frame `{"channelId": "<n>", "message": ...}`
//! like Vibe's `multiplex.py`: a frame for an id above every id seen opens a channel (a lower unknown
//! id is ignored), `message: null` closes one and is acknowledged with the same shape, every channel is
//! its own connection (own `initialize`), and stdin EOF closes every channel. A frame that is not a
//! valid channel frame ends the whole process with code 4 (Vibe's strict outer validation ends the
//! process too), so a client bug shows up at once. Behaviour by flag:
//! - `--ignore-eof`: keep running after stdin EOF (tests the kill-after-grace path).
//! - `--spawn-grandchild`: start a long-sleeping child of itself (tests tree kill / job object).
//! - `--spawn-agent`: start a long-sleeping child in its OWN process group (Windows: new console process
//!   group), like Vibe's `setsid` background processes (an agent-class process for the containment tests).
//! - `--state-file P --crash-from-start K [--crash-until-start M]`: counts starts in P; starts
//!   K..=M (M defaults to forever) exit with code 3 before reading anything.
//! - `--resume-delay-ms MS`: `session/resume` answers after MS milliseconds (in script mode the
//!   connection's reader waits that long, like a harness holding the session lock).
//! - `--stderr-key-at-start`: writes the API key from the env and a traceback to stderr.
//! - `--silent-initialize`: never answers `initialize`.
//! - `--dump-start PATH`: writes `{argv, env, exe, cwd}` of this process to PATH at start (the fixture).
//! - `--mode sleeper`: just sleep 300 s (the grandchild).
//! - `--script [--log-methods PATH] [--log-channels PATH] [--bypass-profile NAME] [--status-events]`: a
//!   scripted Vibe with sessions and an approval turn (see `script.rs`), used by the core API and Debug
//!   API tests (the fixture). `--log-channels` appends `<channel> <method> <sessionId|->` per request
//!   (channel 0 = a single-mode connection), so tests can prove where the core sent each call.
//!   `--rich-turn` plays the richer turn of `rich.rs` (the fixture screenshots).
//!   `--unstored-metadata-miss` acknowledges archive without saving it and returns `not_found`
//!   for delete after a never-run session's channel closes.
//!   `--persist PATH` keeps the sessions in a file across fake processes; `--agent-update-fault
//!   fail:N|wrong:N` makes the first N `session/agent/update` calls fail or report another profile
//!   (`meta.rs`); `--stuck-resumes N`: after such a restart the first N resumes of a session that was
//!   waiting on an approval never answer. `--edit-files`: an approved turn applies the user message's
//!   `edit:<path>` tokens to files in the session folder with `file_write` effects (`edits.rs`, the fixture).
//!   `--subagents-rule` (`subagents.rs`): `config/write` of `/enable_subagents` saves the value
//!   through a new helper chat; `config/fields/read` reports it. `--subagents-reject`:
//!   the write answers `rejected: true` instead. `--delete-faults N`: the first N `session/delete` calls fail.
//!   `VW_FAKE_LOG_AGENT_CONFIG=<file>` (environment, the fixture) appends `{"method", "agentConfig"}` of
//!   every request carrying an `agentConfig` (the MCP server the core injected).
//!
//! Connectors (the fixture): `connector_catalog/*` and `mcp_catalog/*` per `connectors.rs`.
//! Marketplace (the fixture): `plugin_catalog/read`, `plugin/reload`, `skills/installed|setEnabled` per `market.rs`.
//! Worktrees (the fixture): `agentConfig.worktree` on `session/start`, `workspace/git/worktrees/remove|list`
//! per `worktrees.rs` (removal calls are logged).
//!
//! Methods: `initialize`, `echo`, `env/dump` (full environment), `pid/info`, `self/exe` (own
//! executable path and cwd), `session/resume`,
//! `crash/now {code, stderr}` (writes stderr, exits without answering), and a deliberately
//! nonresponsive synthetic `callback/result` path.

mod background;
mod composer;
mod connectors;
mod edits;
mod filesref;
mod handoff;
mod history;
mod market;
mod meta;
mod models;
mod mux;
mod rich;
mod script;
mod subagents;
mod turn;
mod websearch;
mod worktrees;

include!("server_impl.rs");
