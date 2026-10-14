//! Safety reminders for destructive commands. The `pre_tool` hook (`hook.rs`) and the core's own starts
//! (`process_start`, `secret_run`) refuse a command that matches a rule, with a short reason and what to do instead.
//!
//! **Reminders, not a sandbox:** the common honest forms only (no chase after obfuscation; a command the tokenizer
//! cannot read is let through here). The separate kill and git guards check literal command starts.
//! The agent runs as the user by design.
//!
//! Built-in rules ([`RULES`], each can be switched off in Settings > Safety):
//! - `delete-protected`: a recursive delete (`rm -r`, `Remove-Item -Recurse`, `rd /s`, `del /s`) of the home
//!   folder, the filesystem root, a drive root, a top-level system folder, or the session folder or one of its
//!   parents (`..`, an absolute ancestor of `cwd`);
//! - `permissions-protected`: `chmod -R` / `chown -R` / `chgrp -R` on the same places;
//! - `disk-tools`: `mkfs*`, `wipefs`, `fdisk`, `sfdisk`, `gdisk`, `parted`, `diskpart`, `format X:`,
//!   `Format-Volume`, `Clear-Disk`, `Initialize-Disk`, `diskutil eraseDisk|eraseVolume|partitionDisk`, `dd of=/dev/..`;
//! - `shutdown`: `shutdown`, `reboot`, `poweroff`, `halt`, `Stop-Computer`, `Restart-Computer`,
//!   `systemctl poweroff|reboot|halt`, `init 0|6`.
//!
//! Git is the git guard's: pushes and credential commands are refused, common branch rewrites
//! outside the session's own copy ask, and ordinary local changes use Vibe's access-mode card.
//! Not repeated here.
//! Custom rules: "the command contains this text" (case-insensitive), added by the user.
//!
//! Settings: `safety.json` next to the hook endpoint file (user-only; written by the core), read on every check.
//!
//! Depends on: `killparse_shell` (the path/safety tokenizer), serde. Called by: `hook`, vw-core-api
//! (`safety`), tests.
//! Invariants: pure except [`load`] (reads one small file); a missing or unreadable settings file = every built-in
//! rule on, no custom rules.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::killparse::command_name;
use crate::killparse_shell::{Word, segments};

/// One built-in rule.
#[derive(Clone, Copy, Debug)]
pub struct Rule {
    /// Stable id (settings, tests).
    pub id: &'static str,
    /// What it stops, for the Settings list.
    pub label: &'static str,
    /// Why, for the agent.
    pub reason: &'static str,
}

/// The built-in rules, in the order Settings lists them.
pub const RULES: &[Rule] = &[
    Rule {
        id: "delete-protected",
        label: "Deleting your home folder, a drive or system root, or the project's parent folders",
        reason: "this would delete the user's home folder, a whole drive or system folder, or the folder the project \
                 lives in. Delete only the specific files or build folders you created, by their paths inside the \
                 project.",
    },
    Rule {
        id: "permissions-protected",
        label: "Changing permissions or owners of your home folder or a system root, recursively",
        reason: "a recursive chmod/chown there can lock the user out of their own files or break the system. Change \
                 only the specific files that need it.",
    },
    Rule {
        id: "disk-tools",
        label: "Disk tools that erase or repartition drives (mkfs, dd to a device, diskpart, format)",
        reason: "this can erase a whole drive. Disk work is for the user to do themselves.",
    },
    Rule {
        id: "shutdown",
        label: "Shutting down or restarting the computer",
        reason: "this would end the user's work and every running program. Tell the user if a restart is needed.",
    },
];

/// A user's own rule.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomRule {
    /// Stable id.
    pub id: String,
    /// The text a command must contain to be stopped (case-insensitive).
    pub text: String,
}

/// The settings: built-in rules switched off, and custom rules.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SafetySettings {
    /// Ids of built-in rules that are off.
    #[serde(default)]
    pub off: Vec<String>,
    /// Custom rules.
    #[serde(default)]
    pub custom: Vec<CustomRule>,
}

/// Longest custom rule text.
pub const MAX_CUSTOM_TEXT: usize = 200;
/// Most custom rules.
pub const MAX_CUSTOM: usize = 100;

/// The settings file that sits next to the hook endpoint file.
#[must_use]
pub fn settings_file(endpoint_file: &Path) -> PathBuf {
    endpoint_file.with_file_name("safety.json")
}

/// Reads the settings (missing or unreadable: defaults, every rule on).
#[must_use]
pub fn load(path: &Path) -> SafetySettings {
    std::fs::read(path)
        .ok()
        .filter(|b| b.len() <= 256 * 1024)
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// What stopped a command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stop {
    /// The rule id (built-in id, or `custom:<id>`).
    pub rule: String,
    /// The refusal text for the agent.
    pub text: String,
}

fn stop(rule: &Rule) -> Stop {
    Stop {
        rule: rule.id.to_owned(),
        text: format!(
            "Not run (Vibe Work safety reminder: {}): {} If the user really wants this, tell them the exact command \
             so they can run it themselves.",
            rule.label.to_lowercase(),
            rule.reason
        ),
    }
}

/// Checks `command` (run in `cwd`, when known) against the rules that are on. `None` = no rule matches.
#[must_use]
pub fn check(command: &str, cwd: Option<&Path>, settings: &SafetySettings) -> Option<Stop> {
    let lower = command.to_lowercase();
    for custom in &settings.custom {
        let text = custom.text.trim().to_lowercase();
        if !text.is_empty() && lower.contains(&text) {
            return Some(Stop {
                rule: format!("custom:{}", custom.id),
                text: format!(
                    "Not run (Vibe Work safety reminder): the command contains \"{}\", which the user asked agents \
                     not to run. If the user really wants it, tell them the exact command so they can run it \
                     themselves.",
                    custom.text.trim()
                ),
            });
        }
    }
    let on = |id: &str| !settings.off.iter().any(|o| o == id);
    // Backslashes as `/`: Windows paths (`C:\Users\me`) must not read as shell escapes here; only paths matter.
    for segment in segments(&command.replace('\\', "/")) {
        let words = unwrap(&segment.words);
        let Some((first, args)) = words.split_first() else {
            continue;
        };
        let name = command_name(&first.text);
        let hit = match () {
            () if on("delete-protected") && deletes_protected(&name, args, cwd) => {
                "delete-protected"
            }
            () if on("permissions-protected") && chmods_protected(&name, args, cwd) => {
                "permissions-protected"
            }
            () if on("disk-tools") && disk_tool(&name, args) => "disk-tools",
            () if on("shutdown") && shuts_down(&name, args) => "shutdown",
            () => continue,
        };
        return RULES.iter().find(|r| r.id == hit).map(stop);
    }
    None
}

/// The words after leading wrappers (`sudo`, `doas`, `env A=1`, `nohup`, `time`, `command`, `exec`).
fn unwrap(words: &[Word]) -> &[Word] {
    let mut rest = words;
    while let Some((first, tail)) = rest.split_first() {
        let name = command_name(&first.text);
        if matches!(
            name.as_str(),
            "sudo" | "doas" | "nohup" | "time" | "command" | "exec" | "env" | "nice"
        ) {
            rest = tail;
            // Their own options and `NAME=value` words.
            while let Some((w, t)) = rest.split_first() {
                if w.text.starts_with('-') || (name == "env" && w.text.contains('=')) {
                    rest = t;
                } else {
                    break;
                }
            }
        } else if first.text.contains('=')
            && !first.text.starts_with('-')
            && !first.text.starts_with('=')
        {
            rest = tail; // `A=1 rm -rf ~`
        } else {
            break;
        }
    }
    rest
}

fn has_flag(args: &[Word], short: char, long: &[&str]) -> bool {
    args.iter().any(|w| {
        let t = w.text.as_str();
        (t.starts_with('-') && !t.starts_with("--") && t[1..].contains(short))
            || long.iter().any(|l| t.eq_ignore_ascii_case(l))
    })
}

/// True when a PowerShell-style parameter (`-Recurse`, `-r`, `-Rec`) of `name` is among `args`.
fn ps_param(args: &[Word], name: &str, shortest: usize) -> bool {
    args.iter().any(|w| {
        w.text.strip_prefix('-').is_some_and(|p| {
            let p = p.to_ascii_lowercase();
            p.len() >= shortest && name.starts_with(&p)
        })
    })
}

/// True for `-x`, `--x` and a cmd switch (`/s`, `/q`).
fn is_option(t: &str) -> bool {
    t.starts_with('-')
        || ((2..=3).contains(&t.len())
            && t.starts_with('/')
            && t[1..].chars().all(|c| c.is_ascii_alphabetic()))
}

/// The targets: arguments that are not options (`-x`, `/s` for cmd).
fn targets(args: &[Word]) -> impl Iterator<Item = &str> {
    args.iter()
        .map(|w| w.text.as_str())
        .filter(|t| !is_option(t))
}

const SYSTEM_DIRS: &[&str] = &[
    "bin",
    "boot",
    "dev",
    "etc",
    "home",
    "lib",
    "lib64",
    "opt",
    "proc",
    "root",
    "sbin",
    "srv",
    "sys",
    "usr",
    "var",
    "users",
    "applications",
    "library",
    "system",
    "volumes",
    "private",
    "windows",
    "program files",
    "program files (x86)",
    "programdata",
];

/// True when `target` names a protected place (module docs).
fn protected(target: &str, cwd: Option<&Path>) -> bool {
    let mut t = target.trim().replace('\\', "/");
    for suffix in ["/*", "/.*", "/"] {
        while t.len() > 1 && t.ends_with(suffix) {
            t.truncate(t.len() - suffix.len());
            if t.is_empty() {
                t.push('/');
            }
        }
    }
    let lower = t.to_ascii_lowercase();
    let home_words = [
        "~",
        "$home",
        "${home}",
        "%userprofile%",
        "$env:userprofile",
        "$env:home",
        "%homepath%",
    ];
    if t == "/" || home_words.contains(&lower.as_str()) {
        return true;
    }
    if t == ".." || t.starts_with("../..") || t == "../." || lower.starts_with("~/..") {
        return true;
    }
    // A drive root: `C:`, `C:/`.
    let bytes = lower.as_bytes();
    if bytes.len() == 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return true;
    }
    // A top-level system folder (`/usr`, `C:/Windows`) or a user's home (`/home/x`, `/Users/x`, `C:/Users/x`).
    let path = lower
        .strip_prefix(|c: char| c.is_ascii_alphabetic())
        .and_then(|r| r.strip_prefix(':'))
        .unwrap_or(&lower);
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    if path.starts_with('/') {
        match parts.as_slice() {
            [one] if SYSTEM_DIRS.contains(one) => return true,
            [top, _] if matches!(*top, "home" | "users") => return true,
            _ => {}
        }
    }
    // The home folder written out, or the session folder or one of its parents.
    for var in ["HOME", "USERPROFILE"] {
        if let Ok(home) = std::env::var(var)
            && !home.is_empty()
            && same_path(&t, &home)
        {
            return true;
        }
    }
    // `~/x` and `$HOME/x` written out with the home folder, for the parent-of-the-session check.
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_default();
    for prefix in ["~/", "$HOME/", "${HOME}/"] {
        if let Some(rest) = t.strip_prefix(prefix)
            && !home.is_empty()
        {
            t = format!("{}/{rest}", home.replace('\\', "/").trim_end_matches('/'));
            break;
        }
    }
    if let Some(cwd) = cwd
        && (t.starts_with('/') || t.chars().nth(1) == Some(':'))
    {
        let cwd = cwd.to_string_lossy().replace('\\', "/");
        let cwd = cwd.trim_end_matches('/');
        let candidate = t.trim_end_matches('/');
        if !candidate.is_empty() && (same_path(cwd, candidate) || starts_within(cwd, candidate)) {
            return true;
        }
    }
    false
}

fn same_path(a: &str, b: &str) -> bool {
    let (a, b) = (a.replace('\\', "/"), b.replace('\\', "/"));
    let (a, b) = (a.trim_end_matches('/'), b.trim_end_matches('/'));
    if cfg!(any(windows, target_os = "macos")) {
        a.eq_ignore_ascii_case(b)
    } else {
        a == b
    }
}

/// True when `inner` lies inside `outer` (path-wise).
fn starts_within(inner: &str, outer: &str) -> bool {
    let prefix = format!("{}/", outer.trim_end_matches('/'));
    if cfg!(any(windows, target_os = "macos")) {
        inner.to_lowercase().starts_with(&prefix.to_lowercase())
    } else {
        inner.starts_with(&prefix)
    }
}

fn deletes_protected(name: &str, args: &[Word], cwd: Option<&Path>) -> bool {
    let recursive = match name {
        "rm" => has_flag(args, 'r', &["--recursive"]) || has_flag(args, 'R', &[]),
        "remove-item" | "ri" | "del" | "erase" | "rd" | "rmdir" => {
            ps_param(args, "recurse", 2) || args.iter().any(|w| w.text.eq_ignore_ascii_case("/s"))
        }
        _ => return false,
    };
    recursive && targets(args).any(|t| protected(t, cwd))
}

fn chmods_protected(name: &str, args: &[Word], cwd: Option<&Path>) -> bool {
    matches!(name, "chmod" | "chown" | "chgrp")
        && has_flag(args, 'R', &["--recursive"])
        // The first operand is the mode, owner or group.
        && targets(args).skip(1).any(|t| protected(t, cwd))
}

fn disk_tool(name: &str, args: &[Word]) -> bool {
    if name.starts_with("mkfs")
        || matches!(
            name,
            "wipefs"
                | "fdisk"
                | "sfdisk"
                | "gdisk"
                | "cfdisk"
                | "parted"
                | "diskpart"
                | "format-volume"
                | "clear-disk"
                | "initialize-disk"
        )
    {
        return true;
    }
    match name {
        // Windows `format C:` (not `npm run format`, not a git `--format`).
        "format" | "format.com" => args.iter().any(|w| {
            let b = w.text.as_bytes();
            b.len() == 2 && b[0].is_ascii_alphabetic() && b[1] == b':'
        }),
        "diskutil" => args.iter().any(|w| {
            let t = w.text.to_ascii_lowercase();
            matches!(
                t.as_str(),
                "erasedisk"
                    | "erasevolume"
                    | "partitiondisk"
                    | "zerodisk"
                    | "randomdisk"
                    | "secureerase"
            )
        }),
        "dd" => args.iter().any(|w| w.text.starts_with("of=/dev/")),
        _ => false,
    }
}

fn shuts_down(name: &str, args: &[Word]) -> bool {
    match name {
        "shutdown" | "reboot" | "poweroff" | "halt" | "stop-computer" | "restart-computer" => true,
        "systemctl" => args.iter().any(|w| {
            matches!(
                w.text.as_str(),
                "poweroff" | "reboot" | "halt" | "kexec" | "suspend" | "hibernate"
            )
        }),
        "init" | "telinit" => args.iter().any(|w| matches!(w.text.as_str(), "0" | "6")),
        _ => false,
    }
}
