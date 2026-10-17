//! Memory-limited Browser page mounts, persisted preference, and live admissions.
//! Depends on `sysinfo` for memory only; called by `PreviewService` at mount and release boundaries.
//! An admitted session keeps its slot until its engine is closed.
//! Ported from ShellX's browser engine pool.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;
use tokio::time::Instant;

use serde::{Deserialize, Serialize};

/// Browser pool state shown to the user and core.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct EnginePoolSnapshot {
    /// Saved preference: `auto` or a number from `1` to `4`.
    pub configured_parallel_sessions: String,
    /// Current memory-limited admission cap.
    pub effective_cap: u32,
    /// Total usable memory rounded up to GiB, if detected.
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub detected_ram_gb: Option<u64>,
    /// Available memory rounded down to MiB, if detected.
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub free_ram_mb: Option<u64>,
    /// Page keys with an admitted Browser engine (legacy wire field name).
    pub active_sessions: Vec<String>,
    /// Sessions whose last mount request was denied.
    pub waiting_sessions: Vec<String>,
    /// Idle engine release age.
    pub idle_timeout_minutes: u32,
}

/// Reads usable total memory in GiB and available memory in MiB.
/// Hosts may inject a probe; absent readings conservatively admit one engine.
pub type MemoryProbe = Arc<dyn Fn() -> (Option<u64>, Option<u64>) + Send + Sync>;

pub(crate) const IDLE_TIMEOUT: Duration = Duration::from_secs(600);
const SETTINGS_FILE: &str = "engine-pool.json";

#[derive(Default)]
pub(crate) struct PoolState {
    memory_probe: Option<MemoryProbe>,
    pub(crate) configured: String,
    pub(crate) active: HashMap<String, Instant>,
    pub(crate) waiting: HashSet<String>,
    pub(crate) turn_epochs: HashMap<String, u64>,
    pub(crate) pending_turn_starts: HashSet<String>,
    pub(crate) active_turns: HashSet<String>,
    pub(crate) waiting_turns: HashSet<String>,
    pub(crate) saves: HashMap<String, usize>,
    /// Session whose Preview panel is selected, even while a modal parks its native view.
    pub(crate) selected_panel: Option<String>,
}

/// Keeps a user's choice and native save confirmation safe from pool eviction.
/// Dropping the last guard starts a fresh idle grace period for the parked document.
pub struct BrowserSaveGuard {
    pub(crate) pool: Arc<Mutex<PoolState>>,
    pub(crate) session_id: String,
}

impl Drop for BrowserSaveGuard {
    fn drop(&mut self) {
        let mut pool = self.pool.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(count) = pool.saves.get_mut(&self.session_id) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                pool.saves.remove(&self.session_id);
            }
        }
        if let Some(used) = pool.active.get_mut(&self.session_id) {
            *used = Instant::now();
        }
    }
}

impl PoolState {
    pub(crate) fn load(root: &Path, memory_probe: Option<MemoryProbe>) -> Self {
        let configured = std::fs::read_to_string(root.join(SETTINGS_FILE))
            .ok()
            .and_then(|data| serde_json::from_str::<String>(&data).ok())
            .filter(|value| valid_setting(value))
            .unwrap_or_else(|| "auto".to_owned());
        Self {
            memory_probe,
            configured,
            ..Self::default()
        }
    }

    pub(crate) fn snapshot(&self) -> EnginePoolSnapshot {
        let (total, free) = self
            .memory_probe
            .as_ref()
            .map_or_else(detect_memory, |probe| probe());
        let mut active_sessions: Vec<_> = self.active.keys().cloned().collect();
        let mut waiting_sessions: Vec<_> = self.waiting.iter().cloned().collect();
        active_sessions.sort();
        waiting_sessions.sort();
        EnginePoolSnapshot {
            configured_parallel_sessions: self.configured.clone(),
            effective_cap: effective_browser_cap(&self.configured, total, free),
            detected_ram_gb: total,
            free_ram_mb: free,
            active_sessions,
            waiting_sessions,
            idle_timeout_minutes: 10,
        }
    }
}

pub(crate) fn setting_path(root: &Path) -> std::path::PathBuf {
    root.join(SETTINGS_FILE)
}

pub(crate) fn valid_setting(value: &str) -> bool {
    matches!(value, "auto" | "1" | "2" | "3" | "4")
}

/// Computes the memory-limited browser cap from a preference and measured memory.
/// Missing or incomplete memory evidence admits one Browser page engine.
#[must_use]
pub fn effective_browser_cap(configured: &str, total_gb: Option<u64>, free_mb: Option<u64>) -> u32 {
    let (Some(total_gb), Some(free_mb)) = (total_gb, free_mb) else {
        return 1;
    };
    let total_cap = if total_gb >= 28 {
        4
    } else if total_gb >= 16 {
        3
    } else if total_gb >= 8 {
        2
    } else {
        1
    };
    let free_cap = if free_mb < 1536 {
        1
    } else if free_mb < 3072 {
        2
    } else {
        4
    };
    let auto = total_cap.min(free_cap);
    configured
        .parse::<u32>()
        .ok()
        .filter(|n| (1..=4).contains(n))
        .map_or(auto, |fixed| fixed.min(auto))
}

fn detect_memory() -> (Option<u64>, Option<u64>) {
    let mut system = sysinfo::System::new();
    system.refresh_memory();
    let total = system.total_memory();
    if total == 0 {
        return (None, None);
    }
    let free = system.available_memory();
    let (usable_total, usable_free) = (total, free);
    #[cfg(target_os = "linux")]
    let (usable_total, usable_free) = cgroup_memory()
        .map_or((usable_total, usable_free), |(limit, available)| {
            (usable_total.min(limit), usable_free.min(available))
        });
    (
        Some(usable_total.div_ceil(1024 * 1024 * 1024)),
        Some(usable_free / (1024 * 1024)),
    )
}

#[cfg(target_os = "linux")]
fn cgroup_memory() -> Option<(u64, u64)> {
    // The unified controller can limit a nested process below the machine's physical memory.
    let membership = std::fs::read_to_string("/proc/self/cgroup").ok()?;
    let relative = membership
        .lines()
        .find_map(|line| line.strip_prefix("0::"))?;
    let relative = std::path::Path::new(relative.trim_start_matches('/'));
    if relative
        .components()
        .any(|part| !matches!(part, std::path::Component::Normal(_)))
    {
        return None;
    }
    let mut scope = std::path::Path::new("/sys/fs/cgroup").join(relative);
    let root = std::path::Path::new("/sys/fs/cgroup");
    let mut total = u64::MAX;
    let mut available = u64::MAX;
    loop {
        let limit = std::fs::read_to_string(scope.join("memory.max"))
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok());
        let current = std::fs::read_to_string(scope.join("memory.current"))
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok());
        if let (Some(limit), Some(current)) = (limit, current)
            && limit > 0
        {
            total = total.min(limit);
            available = available.min(limit.saturating_sub(current));
        }
        if scope == root {
            break;
        }
        scope = scope.parent()?.to_path_buf();
    }
    (total != u64::MAX).then_some((total, available))
}
