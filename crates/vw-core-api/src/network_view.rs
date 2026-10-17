//! The `network` command's requests and results: Settings > Network reads the proxy and certificate settings Vibe
//! keeps (`config/proxy/read`), tests a way through to Mistral, and saves changes (`config/proxy/write`, with the
//! native confirmation). Logic in `network.rs` and `network_test.rs`; the values and rules in vw-mistral `network`.
//!
//! Depends on: serde. Called by: `command` (the `network` variant), `network`, the UI through the generated TypeScript.
//! Invariants:
//! - Wire form `{"op": "<camelCase>", ...camelCase fields}`, unknown fields refused.
//! - A proxy address leaves the core only with its user name and password hidden (`http://***:***@host:8080`); the
//!   answer says that a sign-in exists (`hasSignIn`), never what it is.
//! - In a request a field that is absent is unchanged, and an empty (or blank) field removes the setting.

use serde::{Deserialize, Serialize};

/// What to do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "op",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum NetworkRequest {
    /// The saved settings, what this computer's environment overrides, and whether a saved change waits for a
    /// restart.
    Read {},
    /// One request to Mistral's model list (no key is sent) with the saved settings and `changes` laid over them,
    /// the way the app would connect. Nothing is saved.
    Test {
        /// The values being tried.
        changes: NetworkChanges,
    },
    /// Checks `changes`, asks for the native confirmation and saves them through Vibe. A value that fails a check
    /// comes back as `refused` and nothing is asked or saved.
    Save {
        /// The values to save.
        changes: NetworkChanges,
    },
}

/// The values of a request. Each is optional: absent is unchanged, empty removes the setting.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NetworkChanges {
    /// `HTTP_PROXY`: the proxy for http requests, `http://host:port` (a sign-in may precede the host).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub http_proxy: Option<String>,
    /// `HTTPS_PROXY`: the proxy for https requests, which is nearly all of Vibe's traffic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub https_proxy: Option<String>,
    /// `ALL_PROXY`: the proxy for any request the two above do not cover.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub all_proxy: Option<String>,
    /// `NO_PROXY`: comma-separated hosts, domains, addresses and ranges that skip the proxy. This computer's own
    /// addresses are always added.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub no_proxy: Option<String>,
    /// `SSL_CERT_FILE`: a PEM file of extra trusted certificates (a company's own certificate authority).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub certificate_file: Option<String>,
    /// `SSL_CERT_DIR`: a folder of extra trusted certificates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub certificate_folder: Option<String>,
}

/// One setting as Settings shows it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct NetworkFieldView {
    /// The saved value; an address with its sign-in hidden. Empty when nothing is saved. For the no-proxy list, only
    /// what the user entered (this computer's own addresses are not listed).
    pub value: String,
    /// The saved address has a user name and password (hidden in `value`). Typing a new address replaces them.
    pub has_sign_in: bool,
    /// This computer's environment sets the same key and wins over the saved value: this is what is in use
    /// (a proxy address with its sign-in hidden); `null` when the environment does not set it.
    pub environment_value: Option<String>,
    /// What is wrong with the saved value right now (a certificate file that is gone or unreadable, an address
    /// that no longer parses), in plain words; `null` when nothing is.
    pub problem: Option<String>,
}

/// All six settings.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct NetworkView {
    /// `HTTP_PROXY`.
    pub http_proxy: NetworkFieldView,
    /// `HTTPS_PROXY`.
    pub https_proxy: NetworkFieldView,
    /// `ALL_PROXY`.
    pub all_proxy: NetworkFieldView,
    /// `NO_PROXY`.
    pub no_proxy: NetworkFieldView,
    /// `SSL_CERT_FILE`.
    pub certificate_file: NetworkFieldView,
    /// `SSL_CERT_DIR`.
    pub certificate_folder: NetworkFieldView,
    /// The addresses that always skip the proxy because Vibe reaches Vibe Work through them.
    pub always_direct: Vec<String>,
    /// What is saved differs from what this run of Vibe Work connects with: it applies the next time Vibe Work
    /// starts (Vibe and the app read the settings once, at their start).
    pub restart_needed: bool,
    /// How many certificates the saved file holds, when it is readable.
    pub certificate_count: Option<u32>,
}

/// Why one value was refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct NetworkProblem {
    /// The request field it is about (`httpsProxy`, `certificateFile`, ...).
    pub field: String,
    /// A plain sentence that never repeats the value.
    pub message: String,
}

/// What the test found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum NetworkTestKind {
    /// Mistral answered.
    Reached,
    /// The proxy could not be reached.
    ProxyUnreachable,
    /// The proxy answered and refused to pass the request on.
    ProxyRefused,
    /// The proxy wants a user name and password.
    ProxySignInNeeded,
    /// The certificate was not trusted.
    CertificateNotTrusted,
    /// Mistral's address could not be found (without a proxy).
    NameNotFound,
    /// The connection could not be made (without a proxy).
    ConnectFailed,
    /// No answer in time.
    TimedOut,
    /// Anything else.
    Failed,
}

/// The result of a test, in words for the screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct NetworkTestView {
    /// What happened.
    pub kind: NetworkTestKind,
    /// True when `kind` is `reached`.
    pub reached: bool,
    /// One plain sentence.
    pub summary: String,
    /// What to check next; `null` when the test passed.
    pub next_step: Option<String>,
    /// The request went through a proxy.
    pub through_proxy: bool,
    /// The settings this computer's environment overrides, which the test used as they are there (for example
    /// `HTTPS proxy`); empty when none.
    pub overridden_by_environment: Vec<String>,
}

/// One `network` answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum NetworkResult {
    /// `read`, and `save` once saved (or when nothing differed).
    Settings {
        /// The settings.
        view: Box<NetworkView>,
        /// `save` only: the settings were written.
        saved: bool,
        /// A short-lived chat the core used and could not delete (its id); it stays in the list as a temporary chat
        /// until Vibe Work quits. `null` otherwise.
        leftover_session_id: Option<String>,
    },
    /// `test`.
    Tested {
        /// What the test found.
        outcome: NetworkTestView,
    },
    /// `test` or `save`: a value failed a check; nothing was tried, asked or saved.
    Refused {
        /// One problem per field.
        problems: Vec<NetworkProblem>,
    },
}
