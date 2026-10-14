//! Small shared pieces of the agent tools: a session's folders, the "inside the session folder" check, owner
//! names for refusals, URLs of listeners and the ports of a process tree.
//!
//! Depends on: vw-procmon, `core::Shared`, `processes` (session contexts), `names`. Called by: `tools`,
//! `start`, `killcheck`.
//! Invariant: owner texts name a session by its title (or id) and another app by its OS executable name only.

use std::net::IpAddr;
use std::path::{Path, PathBuf};

use vw_procmon::{Owner, ProcessRef, Snapshot, StopRefusal, normalize};

use crate::core::Shared;

/// The session's folders (cwd first, then workspace roots), canonicalized in their everyday spelling (Windows:
/// `C:\x`, not `\\?\C:\x`: the agent reads them in `processes_list`, and the File tab refuses a verbatim path the
/// agent repeats in the chat).
/// Errors when the session is not loaded or has none.
pub(crate) fn folders(shared: &Shared, session_id: &str) -> Result<Vec<PathBuf>, String> {
    if let Some(view) = shared.store.view(session_id) {
        crate::session_folders::check(&shared.instructions.own_home, &view.state, None)?;
    }
    let contexts = crate::processes::contexts(&shared.store);
    let Some(context) = contexts.into_iter().find(|c| c.session_id == session_id) else {
        return Err("this session is not open in Vibe Work right now".to_owned());
    };
    let folders: Vec<PathBuf> = context
        .folders
        .iter()
        .map(|f| vw_platform::canonical_plain(f).unwrap_or_else(|_| f.clone()))
        .collect();
    if folders.is_empty() {
        return Err("this session has no folder".to_owned());
    }
    Ok(folders)
}

/// `path` (relative to the first folder, or absolute) resolved and checked to lie inside one of `folders`.
pub(crate) fn inside(folders: &[PathBuf], path: Option<&str>) -> Result<PathBuf, String> {
    let Some(first) = folders.first() else {
        return Err("this session has no folder".to_owned());
    };
    let Some(path) = path.filter(|p| !p.trim().is_empty()) else {
        return Ok(first.clone());
    };
    let joined = if Path::new(path).is_absolute() {
        PathBuf::from(path)
    } else {
        first.join(path)
    };
    // The everyday spelling: this is the `cwd` the agent reads back (and the started program's working folder).
    let resolved = vw_platform::canonical_plain(&joined)
        .map_err(|_| format!("folder {path} does not exist"))?;
    if !resolved.is_dir() {
        return Err(format!("{path} is not a folder"));
    }
    let candidate = normalize(&resolved);
    if folders.iter().any(|f| candidate.starts_with(normalize(f))) {
        Ok(resolved)
    } else {
        Err(format!(
            "folder {path} is outside this session's folder; process_start only runs inside it"
        ))
    }
}

/// Another session's name for refusals: "another session ('Title')", else its id. Always says "another"
/// (session titles can look alike, so the agent needs an unambiguous process owner).
pub(crate) fn session_name(shared: &Shared, session_id: &str) -> String {
    shared
        .names
        .title_of(&shared.store, session_id)
        .map_or_else(
            || format!("another session ({session_id})"),
            |t| format!("another session ('{t}')"),
        )
}

/// "session 'Shop'" / "another app (node)" / "another app".
pub(crate) fn owner_text(shared: &Shared, owner: &Owner) -> String {
    match owner {
        Owner::Session { session_id } => session_name(shared, session_id),
        Owner::OtherApp {
            name: Some(name), ..
        } => format!("another app ({name})"),
        Owner::OtherApp { name: None, .. } => "another app".to_owned(),
    }
}

/// A refusal of the monitor's safe stop, in words the agent can act on.
pub(crate) fn refusal_text(shared: &Shared, refusal: &StopRefusal) -> String {
    match refusal {
        StopRefusal::NotThisSession { pid, owner } => format!(
            "pid {pid} belongs to {}; it was not stopped",
            owner_text(shared, owner)
        ),
        StopRefusal::NotProven { pid, .. } => format!(
            "pid {pid} was not started by this session through process_start (or a terminal of it), so it \
             is not stopped; a Vibe background process is stopped with your own background-process tool"
        ),
        other => format!("{other}; it was not stopped"),
    }
}

/// `http://127.0.0.1:<port>/` for a wildcard or IPv4 loopback listener, `http://[::1]:<port>/` for IPv6.
pub(crate) fn url(address: IpAddr, port: u16) -> String {
    match address {
        IpAddr::V6(v6) if v6.is_loopback() => format!("http://[::1]:{port}/"),
        IpAddr::V4(v4) if !v4.is_unspecified() && !v4.is_loopback() => {
            format!("http://{v4}:{port}/")
        }
        _ => format!("http://127.0.0.1:{port}/"),
    }
}

/// The listening ports (with their address) of `root` and its start-consistent descendants.
pub(crate) fn tree_listeners(snapshot: &Snapshot, root: ProcessRef) -> Vec<(IpAddr, u16)> {
    let mut found: Vec<(IpAddr, u16)> = snapshot
        .listeners
        .iter()
        .filter(|l| {
            l.pid
                .is_some_and(|pid| pid == root.pid || snapshot.descends_from(pid, root))
        })
        .map(|l| (l.address, l.port))
        .collect();
    found.sort_by_key(|(_, p)| *p);
    found.dedup_by_key(|(_, p)| *p);
    found
}

/// True when the process is alive with the same start token.
pub(crate) fn alive(snapshot: &Snapshot, process: ProcessRef) -> bool {
    snapshot
        .alive(process.pid)
        .is_some_and(|p| p.start == Some(process.start))
}
