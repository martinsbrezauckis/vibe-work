//! The marketplace's typed error.
//!
//! Depends on: thiserror. Called by: every module of the crate, vw-core-api (mapped to `CoreError`).
//! Invariant: messages are plain sentences that never contain a download address, a file's content or a secret;
//! a path is named only by its catalog-relative form.

/// What went wrong in the marketplace.
#[derive(Debug, thiserror::Error)]
pub enum MarketError {
    /// The catalog is not readable or breaks a rule.
    #[error("the catalog was refused: {0}")]
    Catalog(String),
    /// An id that is not a plain plugin name (it is never joined to a folder).
    #[error("{0:?} is not a valid marketplace id")]
    InvalidId(String),
    /// No entry with that id.
    #[error("no catalog entry named {0:?}")]
    UnknownEntry(String),
    /// The entry cannot be installed (a server, or no runtime for this platform).
    #[error("{0}")]
    NotInstallable(String),
    /// A download failed (network, status, size).
    #[error("download failed: {0}")]
    Download(String),
    /// Downloaded bytes do not match the catalog's pin: nothing was installed.
    #[error("{what} does not match its pinned hash: nothing was installed")]
    HashMismatch {
        /// Which file or archive (catalog-relative name).
        what: String,
    },
    /// An archive is malformed or holds something the rules refuse (a link, `..`, too large).
    #[error("archive refused: {0}")]
    Archive(String),
    /// A folder exists that the marketplace did not install; it is never touched.
    #[error(
        "a plugin folder named {0:?} exists and was not installed by the marketplace: it was left alone"
    )]
    Foreign(String),
    /// Not installed.
    #[error("{0:?} is not installed")]
    NotInstalled(String),
    /// The post-install step failed or timed out.
    #[error("the setup step failed: {0}")]
    PostInstall(String),
    /// A file system error, with the step that failed.
    #[error("{step}: {source}")]
    Io {
        /// What was being done.
        step: String,
        /// The error.
        #[source]
        source: std::io::Error,
    },
}

impl MarketError {
    /// An I/O error while doing `step`.
    pub fn io(step: impl Into<String>, source: std::io::Error) -> Self {
        Self::Io {
            step: step.into(),
            source,
        }
    }

    /// A short stable code for the UI and the Debug API.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::Catalog(_) => "catalog",
            Self::InvalidId(_) => "invalidId",
            Self::UnknownEntry(_) => "unknownEntry",
            Self::NotInstallable(_) => "notInstallable",
            Self::Download(_) => "download",
            Self::HashMismatch { .. } => "hashMismatch",
            Self::Archive(_) => "archive",
            Self::Foreign(_) => "foreign",
            Self::NotInstalled(_) => "notInstalled",
            Self::PostInstall(_) => "postInstall",
            Self::Io { .. } => "io",
        }
    }
}
