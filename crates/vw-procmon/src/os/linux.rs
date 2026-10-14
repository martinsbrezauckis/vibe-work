//! Linux scan from `/proc`, std only (no `unsafe`, no dependency): `/proc/net/tcp{,6}` for every listening
//! socket with its uid and inode, then one pass over `/proc/<pid>` that reads `stat` for every process and,
//! for processes of our own uid only, `cwd` and the `fd/` links (other users' answer EACCES anyway).
//!
//! Depends on: `procnet`, `types`. Called by: `os::snapshot`, `os::is_alive`.
//! Invariants:
//! - A process or socket that vanishes mid-scan is skipped, never an error. Only an unreadable `/proc`
//!   itself fails the scan.
//! - The `comm` field may contain spaces and parentheses; `stat` is split at its LAST `)`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::time::Instant;

use crate::error::ScanError;
use crate::procnet::parse_proc_net_tcp;
use crate::types::{ProcessInfo, ProcessRef, SelfInfo, Snapshot, SocketListener, StartToken};

/// Fields of `/proc/<pid>/stat` we use.
struct Stat {
    name: String,
    state: char,
    ppid: u32,
    pgid: u32,
    start: u64,
}

fn read_stat(dir: &Path) -> Option<Stat> {
    let raw = fs::read_to_string(dir.join("stat")).ok()?;
    let open = raw.find('(')?;
    let close = raw.rfind(')')?;
    let name = raw.get(open + 1..close)?.to_owned();
    // After ") ": state(3) ppid(4) pgrp(5) session(6) ... starttime(22) => index 19 from field 3.
    let rest: Vec<&str> = raw.get(close + 1..)?.split_whitespace().collect();
    Some(Stat {
        name,
        state: rest.first()?.chars().next()?,
        ppid: rest.get(1)?.parse().ok()?,
        pgid: rest.get(2)?.parse().ok()?,
        start: rest.get(19)?.parse().ok()?,
    })
}

/// Reads `/proc/<pid>/cmdline` (NUL-separated); empty when unreadable or a kernel thread.
pub(crate) fn read_cmdline(pid: u32) -> Vec<String> {
    fs::read(format!("/proc/{pid}/cmdline"))
        .map(|bytes| {
            bytes
                .split(|b| *b == 0)
                .filter(|part| !part.is_empty())
                .map(|part| String::from_utf8_lossy(part).into_owned())
                .collect()
        })
        .unwrap_or_default()
}

/// True when `process` still runs (not a zombie) with the same start token.
pub(crate) fn is_alive(process: ProcessRef) -> bool {
    read_stat(Path::new(&format!("/proc/{}", process.pid)))
        .is_some_and(|s| s.start == process.start.0 && !matches!(s.state, 'Z' | 'X'))
}

/// The identity of `pid` from its `stat` (None when gone or a zombie).
pub(crate) fn identity(pid: u32) -> Option<ProcessRef> {
    read_stat(Path::new(&format!("/proc/{pid}")))
        .filter(|s| !matches!(s.state, 'Z' | 'X'))
        .map(|s| ProcessRef {
            pid,
            start: StartToken(s.start),
        })
}

fn socket_inode(link: &Path) -> Option<u64> {
    let target = fs::read_link(link).ok()?;
    let text = target.to_str()?;
    text.strip_prefix("socket:[")?
        .strip_suffix(']')?
        .parse()
        .ok()
}

/// One full scan.
pub(crate) fn snapshot() -> Result<Snapshot, ScanError> {
    let started = Instant::now();
    let own_uid = fs::metadata("/proc/self")
        .map_err(|e| ScanError::Unreadable(format!("/proc/self: {e}")))?
        .uid();
    let my_pid = std::process::id();
    let mut sockets = Vec::new();
    for file in ["/proc/net/tcp", "/proc/net/tcp6"] {
        // tcp6 is absent when IPv6 is disabled; that is not an error.
        if let Ok(text) = fs::read_to_string(file) {
            sockets.extend(parse_proc_net_tcp(&text));
        }
    }
    let wanted: HashSet<u64> = sockets.iter().map(|s| s.inode).collect();
    let mut owner_of: HashMap<u64, u32> = HashMap::new();
    let mut processes = BTreeMap::new();
    let entries =
        fs::read_dir("/proc").map_err(|e| ScanError::Unreadable(format!("/proc: {e}")))?;
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|n| n.parse::<u32>().ok())
        else {
            continue;
        };
        let dir = entry.path();
        let Some(stat) = read_stat(&dir) else {
            continue;
        };
        let same_user = fs::metadata(&dir).ok().map(|m| m.uid() == own_uid);
        let mut cwd = None;
        let mut exe = None;
        if same_user == Some(true) {
            cwd = fs::read_link(dir.join("cwd")).ok();
            exe = fs::read_link(dir.join("exe")).ok();
            if !wanted.is_empty()
                && let Ok(fds) = fs::read_dir(dir.join("fd"))
            {
                for fd in fds.flatten() {
                    if let Some(inode) = socket_inode(&fd.path()).filter(|i| wanted.contains(i)) {
                        owner_of.entry(inode).or_insert(pid);
                    }
                }
            }
        }
        processes.insert(
            pid,
            ProcessInfo {
                pid,
                ppid: Some(stat.ppid),
                pgid: Some(stat.pgid),
                start: Some(StartToken(stat.start)),
                same_user,
                name: stat.name,
                exe,
                cmdline: Vec::new(),
                cwd,
                alive: !matches!(stat.state, 'Z' | 'X'),
            },
        );
    }
    let listeners: Vec<SocketListener> = sockets
        .iter()
        .map(|s| SocketListener {
            address: s.address,
            port: s.port,
            pid: owner_of.get(&s.inode).copied(),
            same_user: Some(s.uid == own_uid),
        })
        .collect();
    // Command lines only for listener owners (what the views show); `os::fill_cmdline` adds more on demand.
    for pid in listeners.iter().filter_map(|l| l.pid) {
        if let Some(p) = processes.get_mut(&pid)
            && p.cmdline.is_empty()
        {
            p.cmdline = read_cmdline(pid);
        }
    }
    let own_group = processes.get(&my_pid).and_then(|p| p.pgid);
    Ok(Snapshot {
        listeners,
        processes,
        me: SelfInfo {
            pid: my_pid,
            pgid: own_group,
        },
        took: started.elapsed(),
    })
}
