//! Debug API settings and the enablement rule.
//!
//! Depends on: `registry` (UI routes). Called by: the app (builds a config from Settings), tests,
//! `server`.
//! Rule: an explicit user setting wins; without one, the API is on in development builds
//! (`debug_assertions`) and off in user builds. Off means no socket is opened at all.

use std::path::PathBuf;
use std::time::Duration;

use crate::registry::UiRoutes;

/// True in development builds (`debug_assertions`), where the Debug API is on by default.
pub const DEV_BUILD: bool = cfg!(debug_assertions);

/// Why the Debug API is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnabledReason {
    /// A development build and no explicit setting.
    DevBuild,
    /// The user turned it on in Settings.
    UserSetting,
}

impl EnabledReason {
    /// Wire name (`dev-build`, `user-setting`), shown by `/health` and the UI flag.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DevBuild => "dev-build",
            Self::UserSetting => "user-setting",
        }
    }
}

/// Decides whether the API runs. `None` means off.
#[must_use]
pub fn enablement(user_setting: Option<bool>, dev_build: bool) -> Option<EnabledReason> {
    match (user_setting, dev_build) {
        (Some(true), _) => Some(EnabledReason::UserSetting),
        (None, true) => Some(EnabledReason::DevBuild),
        (Some(false), _) | (None, false) => None,
    }
}

/// Everything the app passes in.
#[derive(Clone)]
#[non_exhaustive]
pub struct DebugApiConfig {
    /// The user's Settings switch; `None` when never set (the build default applies).
    pub user_setting: Option<bool>,
    /// Directory for the per-run descriptor file (port and token). The app passes a folder in
    /// its own data directory; it is made user-only (0700 / user-only ACL) before use.
    pub descriptor_dir: PathBuf,
    /// Browser origins allowed to call (exact match). Empty by default: any request with an
    /// `Origin` header is refused. No CORS headers are sent in v1, so even an allowed browser
    /// origin cannot read cross-origin responses; the list exists for the dev mode.
    pub allowed_origins: Vec<String>,
    /// UI-control routes that later blocks register (`/ui/<name>`).
    pub ui_routes: UiRoutes,
    /// Largest accepted request body. Default 1 MiB.
    pub body_limit: usize,
    /// SSE keep-alive comment interval. Default 15 s.
    pub keep_alive: Duration,
    /// Development builds only: override [`DEV_BUILD`] (tests of the user-build default).
    pub dev_build: bool,
    /// The app window for `/windows`, `/screenshot`, `/window`; `None` without a window.
    pub window: Option<crate::window_routes::SharedAppWindow>,
}

impl DebugApiConfig {
    /// A config with defaults and the given descriptor directory.
    #[must_use]
    pub fn new(descriptor_dir: PathBuf) -> Self {
        Self {
            user_setting: None,
            descriptor_dir,
            allowed_origins: Vec::new(),
            ui_routes: UiRoutes::default(),
            body_limit: 1024 * 1024,
            keep_alive: Duration::from_secs(15),
            dev_build: DEV_BUILD,
            window: None,
        }
    }

    /// The enablement for this config.
    #[must_use]
    pub fn enablement(&self) -> Option<EnabledReason> {
        enablement(self.user_setting, self.dev_build)
    }
}
