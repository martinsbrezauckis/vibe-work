//! Error types of the monitor: scans, leases, the ledger. Stop refusals live in `stop` (they are answers,
//! not failures).
//!
//! Depends on: thiserror, vw-platform (`PrivateFileError`). Called by: every module and the core.

/// A scan of the OS tables failed as a whole.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum ScanError {
    /// The process or socket table could not be read.
    #[error("cannot read the OS process tables: {0}")]
    Unreadable(String),
    /// This platform has no scanner.
    #[error("process monitor not supported on this platform")]
    Unsupported,
}

/// Leasing ports failed.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum LeaseError {
    /// The explicitly requested port belongs outside this session's lease.
    #[error("port {port} is not leased to this session; its ports are {leased:?} (port_lease)")]
    NotLeased {
        /// The requested port.
        port: u16,
        /// The session's current ports.
        leased: Vec<u16>,
    },
    /// A listener, pending/live start or the OS prevents use of an explicitly requested port.
    #[error(
        "port {port} is already in use or unavailable to bind; call processes_list and preview_list for listeners and pending/live port assignments, or use another leased port: {leased:?}"
    )]
    Unavailable {
        /// The requested port.
        port: u16,
        /// The session's current ports.
        leased: Vec<u16>,
    },
    /// Every assigned slot is reserved, listening or refused by the OS.
    #[error(
        "no leased port is available ({leased:?}); a start/live process reserves it, a server listens, or the OS refuses binding; call processes_list and preview_list for the current assignments"
    )]
    AllUnavailable {
        /// The session's current ports.
        leased: Vec<u16>,
    },
    /// Not enough free ports left in the range.
    #[error("no free port left in {start}-{end} (wanted {wanted}, found {found})")]
    RangeExhausted {
        /// First port of the range.
        start: u16,
        /// Last port of the range.
        end: u16,
        /// Ports wanted.
        wanted: usize,
        /// Ports found free.
        found: usize,
    },
    /// The socket table could not be read, so nothing is handed out (a guess could collide).
    #[error(transparent)]
    Scan(#[from] ScanError),
}

/// Reading or writing the process ledger failed.
#[derive(Debug, thiserror::Error)]
pub enum LedgerError {
    /// A file operation failed.
    #[error("ledger {path}: {source}")]
    Io {
        /// The file or directory.
        path: String,
        /// The OS error.
        source: std::io::Error,
    },
    /// The directory or a file could not be made user-only.
    #[error(transparent)]
    Private(#[from] vw_platform::PrivateFileError),
    /// A ledger file is not valid JSON of the expected shape (it is left in place, never deleted).
    #[error("ledger {path} is unreadable: {reason}")]
    Corrupt {
        /// The file.
        path: String,
        /// The parse error.
        reason: String,
    },
    /// A scan failed.
    #[error(transparent)]
    Scan(#[from] ScanError),
}
