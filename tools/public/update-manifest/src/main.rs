//! `make-latest-json`: writes or checks the updater's `latest.json` (rules in `src/lib.rs` and `src/files.rs`).
//!
//! ```text
//! make-latest-json write  --version X.Y.Z --app-config <tauri.conf.json> --channel <channel json>
//!                         --pubkey <public key file> --notes-file <file> [--pub-date YYYY-MM-DDTHH:MM:SSZ]
//!                         [--macos-minimum <LSMinimumSystemVersion>] [--windows-minimum <build>]
//!                         [--linux-glibc <glibc version>] --artifacts <dir> --out <latest.json>
//! make-latest-json check  --version X.Y.Z --app-config <tauri.conf.json> --pubkey <public key file>
//!                         [--macos-minimum <v>] [--windows-minimum <v>] [--linux-glibc <v>] --artifacts <dir>
//!                         <latest.json>
//! make-latest-json verify --version X.Y.Z --pubkey <public key file> <file>      (checks <file>.sig)
//! ```
//! Exit codes: 0 done, 1 refused (the reason on stderr), 2 usage.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::path::Path;
use std::process::ExitCode;

use make_latest_json::{Minimums, Refused, WriteRequest};

const USAGE: &str = "usage: make-latest-json write --version X.Y.Z --app-config <file> --channel <file> \
--pubkey <file> --notes-file <file> [--pub-date <time>] [--macos-minimum <version>] [--windows-minimum <build>] \
[--linux-glibc <version>] --artifacts <dir> --out <file>\n       \
make-latest-json check --version X.Y.Z --app-config <file> --pubkey <file> [--macos-minimum <version>] \
[--windows-minimum <build>] [--linux-glibc <version>] --artifacts <dir> <latest.json>\n       \
make-latest-json verify --version X.Y.Z --pubkey <file> <file>";

/// Parsed `--flag value` pairs plus the positional arguments; `None` on a flag without a value or given twice.
fn parse(args: impl Iterator<Item = String>) -> Option<(BTreeMap<String, String>, Vec<String>)> {
    let mut flags = BTreeMap::new();
    let mut positional = Vec::new();
    let mut args = args;
    while let Some(arg) = args.next() {
        if let Some(name) = arg.strip_prefix("--") {
            let value = args.next()?;
            if flags.insert(name.to_owned(), value).is_some() {
                return None;
            }
        } else {
            positional.push(arg);
        }
    }
    Some((flags, positional))
}

/// Runs one command; `None` = usage error.
fn run(
    command: &str,
    flags: &BTreeMap<String, String>,
    positional: &[String],
) -> Option<Result<String, Refused>> {
    let flag = |name: &str| flags.get(name).map(String::as_str);
    let minimums = Minimums {
        macos: flag("macos-minimum"),
        windows: flag("windows-minimum"),
        linux_glibc: flag("linux-glibc"),
    };
    let known: &[&str] = match command {
        "write" => &[
            "version",
            "app-config",
            "channel",
            "pubkey",
            "notes-file",
            "pub-date",
            "macos-minimum",
            "windows-minimum",
            "linux-glibc",
            "artifacts",
            "out",
        ],
        "check" => &[
            "version",
            "app-config",
            "pubkey",
            "macos-minimum",
            "windows-minimum",
            "linux-glibc",
            "artifacts",
        ],
        "verify" => &["version", "pubkey"],
        _ => return None,
    };
    if flags.keys().any(|k| !known.contains(&k.as_str())) {
        return None;
    }
    Some(match command {
        "write" => {
            if !positional.is_empty() {
                return None;
            }
            let request = WriteRequest {
                version: flag("version")?,
                app_config: Path::new(flag("app-config")?),
                channel: Path::new(flag("channel")?),
                pubkey: Path::new(flag("pubkey")?),
                notes: Path::new(flag("notes-file")?),
                pub_date: flag("pub-date"),
                minimums,
                artifacts: Path::new(flag("artifacts")?),
                out: Path::new(flag("out")?),
            };
            make_latest_json::write_manifest(&request).map(|out| format!("wrote {}", out.display()))
        }
        "check" => {
            let [manifest] = positional else { return None };
            make_latest_json::check_manifest(
                flag("version")?,
                Path::new(flag("app-config")?),
                Path::new(flag("pubkey")?),
                Path::new(flag("artifacts")?),
                minimums,
                Path::new(manifest),
            )
            .map(|()| format!("{manifest}: ok"))
        }
        _ => {
            let [file] = positional else { return None };
            let version = flag("version")?;
            make_latest_json::verify_file(version, Path::new(flag("pubkey")?), Path::new(file))
                .map(|()| format!("{file}: the signature verifies (version {version})"))
        }
    })
}

fn main() -> ExitCode {
    let mut raw = std::env::args().skip(1);
    let command = raw.next().unwrap_or_default();
    let outcome = parse(raw).and_then(|(flags, positional)| run(&command, &flags, &positional));
    match outcome {
        None => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
        Some(Ok(line)) => {
            println!("{line}");
            ExitCode::SUCCESS
        }
        Some(Err(refusal)) => {
            eprintln!("refused: {refusal}");
            ExitCode::from(1)
        }
    }
}
