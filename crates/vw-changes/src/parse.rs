//! The record of one changed path (built by `diff` from our own baseline entries and working-tree reading,
//! never from tree objects the agent could rewrite) and the stable hash for hunk ids and fingerprints.
//!
//! Depends on: std. Called by: `diff`.

/// One changed path (the shape of a `git diff --raw` record).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawEntry {
    /// Baseline mode (`000000` when absent).
    pub old_mode: String,
    /// Current mode (`000000` when absent).
    pub new_mode: String,
    /// Baseline blob id.
    pub old_id: String,
    /// Current blob id (all zeros when absent).
    pub new_id: String,
    /// Status letter (`A`, `M`, `D`, `R`, `T`, ...).
    pub status: char,
    /// Rename/copy similarity (0 otherwise).
    pub score: u32,
    /// Baseline path as git's bytes (the source of a rename; the path otherwise).
    pub old_path: Vec<u8>,
    /// Current path as git's bytes.
    pub path: Vec<u8>,
}

/// 64-bit FNV-1a, as hex: stable across runs and platforms (hunk ids, fingerprints).
#[must_use]
pub fn fnv_hex(parts: &[&[u8]]) -> String {
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
