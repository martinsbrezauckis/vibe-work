//! The command line of the `vw-agent-tools` binary, in the library so the private tests can build
//! the same program as a test binary (`CARGO_BIN_EXE_*` only exists for a package's own binaries).
//!
//! Modes:
//! - Named `gh`: credential-free client for the app's GitHub read broker.
//! - Named `bash` (Windows): the real Git Bash in a kill-on-close job, so Stop ends everything it started (`git_bash`).
//! - Named `pwsh` or `powershell` (Windows): the same for the real PowerShell (`powershell`).
//! - `gh-broker <real gh>`: trusted app-only mode, token over stdin and endpoint over stdout.
//! - `mcp`: the per-session stdio MCP server (`agentConfig.mcpServers`, name `vw`). Reads the core endpoint and
//!   the session credential from `VW_AGENT_ENDPOINT` / `VW_AGENT_CREDENTIAL` (set by the core, never in argv).
//!   `--chrome-catalog` advertises the Chrome tools (the core decides it per chat), `--without-chrome` marks a build
//!   without the Chrome package; with neither flag no Chrome tool is listed; `--app-version <version>` is accepted and
//!   ignored, as is `--build-id <identity>` (they separate tool-list caches per build). Exits when stdin closes.
//! - `guard <safety|process|git|folder|connector|receipt|allowed> --endpoint-file <path>`: a single check from
//!   `$VIBE_HOME/hooks.toml`. The filename can instead arrive through `VW_HOOK_ENDPOINT_FILE` (Windows).
//!   Reads Vibe's hook payload on stdin, prints a deny decision or nothing, exits 0.
//! - `kill-guard --endpoint-file <path>`: the managed pre-tool hook: selected guards in order, with the refusing guard named in the reason.
//!
//! Depends on: `hook`, `mcp`, `wire`. Called by: `main.rs`, the private test binary.

use std::io::{BufReader, Read};
use std::path::PathBuf;
use std::process::ExitCode;

use crate::{hook, mcp, wire};

fn mcp_mode(chrome: bool, package: bool) -> ExitCode {
    // This process holds its chat's tool credential (environment and memory). On Linux another
    // same-user program, such as another chat's Full-access agent, could read it from /proc and act as this chat
    // (and so under this chat's project vault permissions). Non-dumpable, like the core, closes those reads.
    #[cfg(target_os = "linux")]
    let _ = rustix::process::set_dumpable_behavior(rustix::process::DumpableBehavior::NotDumpable);
    let endpoint = std::env::var(wire::ENV_ENDPOINT).ok();
    let credential = std::env::var(wire::ENV_CREDENTIAL).ok();
    let forward = move |tool: &str, args: serde_json::Value| match (&endpoint, &credential) {
        // One budget for every tool, below Vibe's limit for a session MCP tool call: a tool that waits for a
        // person answers Waiting within it and the agent collects the outcome with that tool's status call.
        (Some(endpoint), Some(credential)) => {
            wire::call(endpoint, credential, tool, args, wire::CALL_TIMEOUT)
        }
        _ => Err("Vibe Work's tools are not connected in this session (no credential).".to_owned()),
    };
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    match mcp::serve_with_package(
        BufReader::new(stdin.lock()),
        stdout.lock(),
        forward,
        chrome,
        package,
    ) {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::from(1),
    }
}

fn kill_guard(args: &[String], guard: Option<hook::Guard>) -> ExitCode {
    let file = endpoint_file(args, std::env::var_os(hook::ENV_ENDPOINT_FILE));
    let mut payload = String::new();
    let limit = u64::try_from(hook::MAX_PAYLOAD).unwrap_or(u64::MAX);
    if std::io::stdin()
        .take(limit)
        .read_to_string(&mut payload)
        .is_err()
    {
        payload.clear();
    }
    let decision = match guard {
        Some(guard) => guard.decide(&payload, &file),
        None => hook::decide(&payload, &file),
    };
    if let Some(text) = decision {
        println!("{text}");
    }
    ExitCode::SUCCESS
}

fn endpoint_file(args: &[String], inherited: Option<std::ffi::OsString>) -> PathBuf {
    args.iter()
        .position(|a| a == "--endpoint-file")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from)
        .or_else(|| inherited.map(PathBuf::from))
        .unwrap_or_default()
}

/// Runs the program with `args` (without the program name).
#[must_use]
pub fn run(args: &[String]) -> ExitCode {
    let name = std::env::args_os()
        .next()
        .and_then(|p| PathBuf::from(p).file_stem().map(std::ffi::OsStr::to_owned));
    if name
        .as_ref()
        .is_some_and(|name| name.eq_ignore_ascii_case("bash"))
    {
        return crate::git_bash::run();
    }
    if name
        .as_ref()
        .is_some_and(|name| crate::powershell::is_mode_name(name))
    {
        return crate::powershell::run();
    }
    if name.is_some_and(|name| name.eq_ignore_ascii_case("gh")) {
        return match vw_gh_broker::entry::shim() {
            Ok(code) => {
                std::process::exit(code);
            }
            Err(error) => {
                eprintln!("{error}");
                ExitCode::from(1)
            }
        };
    }
    match args.first().map(String::as_str) {
        Some("gh-broker") => {
            if args
                .get(1)
                .map(PathBuf::from)
                .ok_or_else(|| std::io::Error::other("missing gh path"))
                .and_then(|path| vw_gh_broker::entry::server(&path))
                .is_ok()
            {
                ExitCode::SUCCESS
            } else {
                eprintln!("GitHub reader stopped.");
                ExitCode::from(1)
            }
        }
        Some("mcp") => mcp_mode(
            args.iter()
                .skip(1)
                .any(|arg| arg == wire::CHROME_CATALOG_ARG),
            !args.iter().any(|arg| arg == wire::WITHOUT_CHROME_ARG),
        ),
        Some("kill-guard") => kill_guard(&args[1..], None),
        Some("guard") => {
            if let Some(guard) = args.get(1).and_then(|name| hook::Guard::from_arg(name)) {
                kill_guard(&args[2..], Some(guard))
            } else {
                // An unknown guard name (a newer hooks.toml?) refuses rather than silently allowing: print a deny.
                println!(
                    "{}",
                    serde_json::json!({"decision": "deny", "reason": "Not run: Vibe Work's guard is misconfigured (unknown guard name). Ask the user to restart Vibe Work."})
                );
                ExitCode::SUCCESS
            }
        }
        _ => {
            eprintln!(
                "usage: vw-agent-tools mcp | vw-agent-tools guard <safety|process|git|folder|connector|receipt|allowed> --endpoint-file <path> | vw-agent-tools kill-guard --endpoint-file <path>"
            );
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_endpoint_environment_is_literal_and_explicit_file_takes_precedence() {
        let path = r"C:\OneDrive - Mārtiņš\$`';&\hook.json";
        assert_eq!(endpoint_file(&[], Some(path.into())), PathBuf::from(path));
        assert_eq!(
            endpoint_file(
                &["--endpoint-file".into(), "explicit.json".into()],
                Some(path.into())
            ),
            PathBuf::from("explicit.json")
        );
        assert_eq!(endpoint_file(&[], None), PathBuf::new());
    }
}
