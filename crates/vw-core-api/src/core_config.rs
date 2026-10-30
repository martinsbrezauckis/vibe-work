//! Core startup settings and production defaults. Depends on the subsystem configuration types;
//! used by `Core` and its callers so every core starts with the same policy and timing defaults.

use std::sync::Arc;
use std::time::Duration;

use vw_gate::GateConfig;
use vw_terminal::{PortLeaseSource, TerminalConfig};

/// Tuning of a core. `Default` gives production values.
#[derive(Clone)]
#[non_exhaustive]
pub struct CoreConfig {
    /// Shared clock for permission deadlines, replaceable in tests.
    pub ask_clock: Arc<dyn crate::AskClock>,
    /// Idle-sleep OS edge, replaceable by tests. One worker owns the app-wide request.
    pub keep_awake_backend: Arc<dyn vw_platform::keep_awake::IdleSleepBackend>,
    /// App data folder for scan settings and incremental state. No UI or Vault values are stored here.
    pub secret_scan_data: Option<std::path::PathBuf>,
    /// The gate's tuning (token lifetime, limits, clock).
    pub gate: GateConfig,
    /// How long a command waits for a ready server before failing with `serverUnavailable`.
    /// Default 10 s.
    pub connection_wait: Duration,
    /// Undelivered store items kept per subscriber before it is resnapshotted. Default 1024.
    pub subscriber_capacity: usize,
    /// Core events buffered per subscriber before it is told it lagged. Default 256.
    pub event_capacity: usize,
    /// Stderr lines in a server report. Default 40.
    pub stderr_lines: usize,
    /// Commands the user types in the app's own window (`session/shellCommand` from
    /// [`Origin::Ui`](crate::Origin)) and the typed composer Full access switch run without the native dialog,
    /// because the main window is a trusted surface that never renders untrusted active content.
    /// Trusted-surface tests cover this requirement. Default `true`. Other methods keep the window's dialog
    /// regardless. The Debug API is not the window: the core's confirmer answers its confirmations itself
    /// (every one but the vault's), Full access included.
    pub trusted_ui_shell: bool,
    /// How long a session's channel stays open after its last use when the session is quiet and no
    /// client views it (`recovery.rs`). Closing releases the session's lease in Vibe, so the
    /// Vibe CLI can take the session, and frees the channel's memory in the server; the next command
    /// for the session reopens it (`session/resume`, usually well under a second).
    /// Default 5 minutes: long enough that switching between a few sessions never reopens them, short
    /// enough that a session left alone is free for the CLI soon.
    pub channel_idle_close: Duration,
    /// Longest wait for reopening a session after a server restart; past it (or on an error) the
    /// session is stuck: its commands are refused until the user acts (no automatic retries, a user must
    /// never wait minutes mid-work). Default 60 s: room for a slow reopen right after a restart.
    pub reopen_timeout: Duration,
    /// The same wait for a session whose turn was cut off by the restart: Vibe re-runs that turn inside
    /// `session/resume` (model rounds and the interrupted command) and answers only after it, so a healthy reopen can
    /// take minutes. Default 150 s, the supervisor's own resume bound.
    pub inflight_reopen_timeout: Duration,
    /// Longest wait for one `retryReopen` attempt. Default 60 s.
    pub retry_reopen_timeout: Duration,
    /// How often the foreign-holder poll reads the viewed sessions no channel of ours holds
    /// ([`crate::session_holder`]). Default 2 s. A test that counts the exact Vibe calls of one command sets it
    /// long, since a poll read landing in its window is not part of that command.
    pub holder_poll: Duration,
    /// The key service: answers `keys` commands; its events become `keyNeeded` / `keyStatusChanged` core events.
    /// `None` (tests, runs with the key in the environment): `keys` commands fail with `keyService` / `notConfigured`.
    pub keys: Option<Arc<vw_keys::KeyService>>,
    /// Native clipboard owner for user-only vault copy commands. The core shuts it down.
    pub clipboard: Option<Arc<vw_platform::clipboard::ClipboardService>>,
    /// The Vibe install at setup: answers `vibe` commands; its view changes become `vibeInstall`
    /// events. When it has no server yet (`VibeInstaller::server` is `None`), the core starts with the
    /// supervisor in `notInstalled` (nothing spawned) and starts the first server when the install finishes.
    /// `None` (tests, development builds): `vibe` commands fail with `notConfigured`.
    pub vibe_install: Option<vw_vibe_install::VibeInstaller>,
    /// Start Vibe when the core starts (default true). Tests of core-only screens can keep the supervisor pending
    /// without starting any server or replacing Vibe's protocol; installer state remains available independently.
    pub start_vibe: bool,
    /// Process and port monitor settings: lease range, stop timing, tick, ledger folder.
    /// `Default` has no ledger; the app sets `ledger_dir` to a folder in its data directory.
    pub procmon: vw_procmon::ProcMonConfig,
    /// Integrated terminals: per-session cap, scrollback, flow control, shell.
    pub terminals: TerminalConfig,
    /// Where a session's terminals get `PORT` from. Default `None`: the process monitor's
    /// per-session lease (the first leased port). Tests may set a fixed source.
    pub terminal_ports: Option<Arc<dyn PortLeaseSource>>,
    /// Instructions: where the user's command-line home is, for the copy offer.
    pub instructions: crate::instructions::InstructionsConfig,
    /// Documents: how long one OCR request may take.
    pub documents: crate::documents::DocumentsConfig,
    /// Whether the native app window can receive a one-time reading. None means no listening window.
    pub read_aloud_available: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
    /// Voice: dictation length, idle drop, how long `stop` waits for the live text.
    pub speech: crate::dictation::SpeechConfig,
    /// Session-scoped agent tools: the per-session MCP server, `process_start` and the kill
    /// guard's endpoint. `None` (default): no MCP server is registered.
    pub agent_tools: Option<crate::agent_tools::AgentToolsConfig>,
    /// Where the Changes tab keeps review baselines (made user-only). `None` (the default): the
    /// Changes tab is off. The app sets a folder in its data directory.
    pub changes_dir: Option<std::path::PathBuf>,
    /// Longest a session's review baseline may take before its first piece of work is sent to Vibe without
    /// one. A baseline still running at this point is discarded when it ends (taken late, it could
    /// contain the agent's first edits) and the review is reported unavailable until the user starts it
    /// again. Default 15 s.
    pub changes_baseline_limit: Duration,
    /// Test switch: calls around the baseline taken before a session's first work, so a test can
    /// hold it past `changes_baseline_limit` and know when it ended. Default `None`.
    pub changes_test_hooks: Option<Arc<dyn crate::changes_slots::ChangesTestHooks>>,
    /// The preview pane: the native engine and its settings. `None` (the default): no previews.
    pub preview: Option<crate::previews::PreviewSetup>,
    /// Project Pages: the viewer engine and the app-data folders. `None` (the default): no Pages.
    pub pages: Option<crate::pages::PagesSetup>,
    /// The user's project groups: the folder of `projects.json`. `None` (the default): the groups live
    /// in memory for this run only. The app sets a folder in its data directory.
    pub projects_dir: Option<std::path::PathBuf>,
    /// Session bookkeeping of Vibe Work's own: the temporary sessions' record. `None` (the default):
    /// memory only, so temporary sessions of a crashed run are not deleted at the next start.
    pub sessions_dir: Option<std::path::PathBuf>,
    /// The Vibe CLI's session folder when this run's Vibe saves chats there (chats shared with the CLI). `None`
    /// (the default): chats are kept separate, so `vibe --continue` in a terminal resumes none of them and the
    /// listing names no chat for it (`session_listing`).
    pub cli_session_dir: Option<std::path::PathBuf>,
    /// The File tab's viewer for HTML pages and PDFs: a third engine instance whose config uses
    /// `vw_pages::FILES_LABEL_PREFIX`. `None` (the default): HTML and PDF files show their source / a notice only.
    pub files_viewer: Option<crate::pages::PagesSetup>,
    /// The version this build reports (Settings > About, `GET /health`, `ui/state`). The app
    /// passes Tauri's `package_info().version`, the version the updater compares, so all of them agree (the
    /// release scripts set it in the Tauri config overlay; the Cargo workspace version stays `0.0.0`). `None`
    /// (tests, the default): this crate's own version.
    pub app_version: Option<String>,
    /// gh to use for pull requests and the PR card. `None` (the default): the first `gh` on `PATH` or
    /// in the usual install folders. Tests point it at a fake.
    pub gh_program: Option<std::path::PathBuf>,
    /// GitHub's REST API base for the PR card without gh. Default `https://api.github.com`; tests
    /// point it at a local fake.
    pub github_api_base: String,
    /// The model that drafts commit messages. Default [`crate::git_cmd::DEFAULT_COMMIT_MODEL`].
    pub commit_model: String,
    /// The shortest time between two forge reads of one PR card. `None` (the default): 30 s with gh,
    /// 120 s with GitHub's public API. Tests set it to zero.
    pub pr_card_floor: Option<Duration>,
    /// Where the core keeps the proxy and certificate settings saved in Settings > Network (vw-mistral
    /// `network::settings_file` in app data; authenticated with the key service's vault after confirmed Save).
    /// Launch verifies this agent-writable file, never Vibe's `.env`. `None` (tests, runs without a data
    /// folder): Settings shows nothing saved and a save is refused.
    pub network_file: Option<std::path::PathBuf>,
    /// Launch rejected the network record; replay its security notice to the window.
    pub network_settings_rejected: bool,
    /// The address the Settings > Network test requests once (no key is sent). Default Mistral's model list, which
    /// answers 401 without a key: that answer proves the way through works. Tests point it at a local fake.
    pub network_test_url: String,
    /// The Tools marketplace: a test catalog and downloader; the default is the built-in catalog over HTTPS.
    pub market: crate::market::MarketConfig,
    /// Chrome Control's package, store extension id and the user's Chrome places (`chrome_control::setup`).
    pub chrome_control: crate::ChromeControlConfig,
    /// Libraries and image generation: `work.json`'s folder, the models, the image download policy.
    pub work: crate::work::WorkConfig,
}

impl CoreConfig {
    /// How often the pump looks for idle channels: a quarter of `channel_idle_close`, between 10 ms
    /// (tests) and 30 s.
    pub(crate) fn idle_check(&self) -> Duration {
        (self.channel_idle_close / 4).clamp(Duration::from_millis(10), crate::pump::MAX_IDLE_CHECK)
    }
}

impl Default for CoreConfig {
    fn default() -> Self {
        Self {
            ask_clock: Arc::new(crate::SystemAskClock::default()),
            keep_awake_backend: Arc::new(vw_platform::keep_awake::NativeIdleSleep),
            secret_scan_data: None,
            gate: GateConfig::default(),
            connection_wait: Duration::from_secs(10),
            subscriber_capacity: 1024,
            event_capacity: 256,
            stderr_lines: 40,
            trusted_ui_shell: true,
            channel_idle_close: Duration::from_secs(300),
            reopen_timeout: Duration::from_secs(60),
            inflight_reopen_timeout: Duration::from_secs(150),
            retry_reopen_timeout: Duration::from_secs(60),
            holder_poll: crate::session_holder::POLL,
            keys: None,
            clipboard: None,
            vibe_install: None,
            start_vibe: true,
            procmon: vw_procmon::ProcMonConfig::default(),
            terminals: TerminalConfig::default(),
            terminal_ports: None,
            instructions: crate::instructions::InstructionsConfig::default(),
            documents: crate::documents::DocumentsConfig::default(),
            speech: crate::dictation::SpeechConfig::default(),
            read_aloud_available: None,
            agent_tools: None,
            changes_dir: None,
            changes_baseline_limit: crate::changes::BASELINE_LIMIT,
            changes_test_hooks: None,
            preview: None,
            pages: None,
            projects_dir: None,
            sessions_dir: None,
            cli_session_dir: None,
            files_viewer: None,
            app_version: None,
            gh_program: None,
            github_api_base: "https://api.github.com".to_owned(),
            commit_model: crate::git_cmd::DEFAULT_COMMIT_MODEL.to_owned(),
            pr_card_floor: None,
            network_file: None,
            network_settings_rejected: false,
            network_test_url: format!("{}/v1/models", vw_mistral::DEFAULT_API_BASE),
            market: crate::market::MarketConfig::default(),
            chrome_control: crate::ChromeControlConfig::default(),
            work: crate::work::WorkConfig::default(),
        }
    }
}
