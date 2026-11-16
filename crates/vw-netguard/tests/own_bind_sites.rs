//! Every TCP listener the product binds is one of Vibe Work's own ports: product code (every crate's `src/` and the
//! app's `src/`, test modules and test files left out) binds a TCP listener only through
//! `vw_netguard::listeners::bind_own` / `bind_own_tokio`, which put the port in the own-listener set before the
//! listener is handed out. The guard's own listeners register themselves (`guard.rs`, `listeners::set`); a free-port
//! probe that closes the socket at once is not a listener. A new `TcpListener::bind` anywhere else fails here, so a
//! new server cannot be left reachable (or asked about) from a web page.
//!
//! PREDICT (before the bind sites moved to `bind_own*`): fails listing the page server, Debug API, key proxy, agent
//! tools endpoint, process tracker and single-instance listeners, each with its own `TcpListener::bind`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

/// Files whose binds are allowed, with the reason.
const ALLOWED: &[(&str, &str)] = &[
    (
        "crates/vw-netguard/src/listeners.rs",
        "the own-listener helper itself",
    ),
    (
        "crates/vw-netguard/src/guard.rs",
        "the guard's shared and store listeners (registered through listeners::set)",
    ),
    (
        "crates/vw-procmon/src/leases.rs",
        "a free-port probe: the socket is dropped at once, never accepts",
    ),
];

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            if name != "tests" && name != "target" {
                rust_files(&path, out);
            }
        } else if path.extension().is_some_and(|e| e == "rs")
            && !name.ends_with("_tests.rs")
            && name != "tests.rs"
            && !name.starts_with("test_")
        {
            out.push(path);
        }
    }
}

/// The product lines of a source file (1-based numbers): an inline `#[cfg(test)] mod name {` and everything after
/// it is left out (inline test modules sit at the end); `#[cfg(test)] mod name;` declarations cut nothing.
fn product_lines(text: &str) -> Vec<(usize, &str)> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    for (at, line) in lines.iter().enumerate() {
        let next = lines.get(at + 1).map_or("", |l| l.trim());
        if line.trim() == "#[cfg(test)]" && next.starts_with("mod ") && next.ends_with('{') {
            break;
        }
        out.push((at + 1, *line));
    }
    out
}

#[test]
fn every_product_tcp_listener_binds_through_the_own_listener_helper() {
    let root = workspace();
    let mut files = Vec::new();
    for entry in std::fs::read_dir(root.join("crates")).unwrap().flatten() {
        rust_files(&entry.path().join("src"), &mut files);
    }
    rust_files(&root.join("app/src"), &mut files);
    assert!(
        files.len() > 100,
        "source tree not found under {}",
        root.display()
    );
    let mut found = Vec::new();
    for file in files {
        let relative = file
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if ALLOWED.iter().any(|(path, _)| *path == relative) {
            continue;
        }
        let text = std::fs::read_to_string(&file).unwrap();
        for (number, line) in product_lines(&text) {
            if line.contains("Listener::bind(") && !line.trim_start().starts_with("//") {
                found.push(format!("{relative}:{number}: {}", line.trim()));
            }
        }
    }
    assert!(
        found.is_empty(),
        "product listeners bound outside vw_netguard::listeners::bind_own*: a web page could reach them, or the \
         person would be asked about them:\n{}",
        found.join("\n")
    );
}
