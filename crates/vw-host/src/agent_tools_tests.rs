//! Hook command regression tests. Uses the host writer and real shells; paths must remain literal in either shell.
use super::{Path, hook_command};

#[cfg(unix)]
const NASTY_PATHS: &[&str] = &[
    "a b",
    "it's",
    "a\"quote",
    "$cash",
    "`tick",
    ";echo injected",
    "a&b",
    "Mārtiņš",
    "OneDrive - Company",
    "$(echo injected)",
    "[brackets]",
    "all $`';& together",
    "left‘quote",
    "right’quote",
    "low‚quote",
    "reversed‛quote",
    "smart’$(echo injected)",
    "double“quote”",
    "‘’“”'",
    "mixed ‘left’ and “right” it's",
    "’`$() ; & ' “quoted”",
];

#[test]
fn endpoint_environment_is_excluded_from_msys_path_conversion() {
    for (existing, expected) in [
        (None, "VW_HOOK_ENDPOINT_FILE="),
        (Some("OTHER="), "OTHER=;VW_HOOK_ENDPOINT_FILE="),
        (Some("*"), "*"),
    ] {
        let endpoint = Path::new(r"C:\a b\$`';& Mārtiņš\hook.json");
        let env = super::hook_environment(endpoint, existing.map(Into::into));
        assert_eq!(env[0].1, endpoint.as_os_str());
        assert_eq!(
            env.iter()
                .find(|(name, _)| name == "MSYS2_ENV_CONV_EXCL")
                .map(|(_, value)| value.as_os_str()),
            Some(std::ffi::OsStr::new(expected))
        );
    }
}

#[test]
fn windows_hooks_invoke_the_absolute_binary_without_a_launcher() {
    let line = hook_command(
        Path::new(r"C:\OneDrive - Mārtiņš\a b\$cash`tick'quote;&\vw-agent-tools.exe"),
        Path::new(r"C:\data\$cash`tick'quote;&\hook.json"),
        "kill-guard",
        true,
    );
    assert!(!line.contains("powershell"), "hook still starts PowerShell");
    assert_eq!(
        line,
        r#"C:/'OneDrive - Mārtiņš/a b/$cash`tick'"'"'quote;&/vw-agent-tools.exe' kill-guard"#
    );
}

#[cfg(unix)]
#[test]
fn windows_line_preserves_nasty_paths_in_bash_and_powershell() {
    use std::os::unix::fs::PermissionsExt as _;
    use std::process::Command;

    let root = tempfile::tempdir().unwrap();
    let mut shells = vec![("/bin/bash".into(), vec!["-c"])];
    if let Some(pwsh) = std::env::var_os("VW_TEST_POWERSHELL") {
        shells.push((
            pwsh,
            vec!["-NoLogo", "-NoProfile", "-NonInteractive", "-Command"],
        ));
    }
    for name in NASTY_PATHS {
        let dir = root.path().join(name);
        std::fs::create_dir(&dir).unwrap();
        let binary = dir.join("vw-agent-tools");
        std::fs::write(
            &binary,
            "#!/bin/sh\nprintf '%s\\n' \"$*\" \"$VW_HOOK_ENDPOINT_FILE\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        let endpoint = dir.join("hook.json");
        let line = hook_command(&binary, &endpoint, "kill-guard", true);
        // The command reaches Vibe through a TOML basic string before either shell parses it.
        let saved: toml::Value =
            toml::from_str(&format!("command = {}", super::toml_string(&line))).unwrap();
        let line = saved["command"].as_str().unwrap();
        for (shell, flags) in &shells {
            let output = Command::new(shell)
                .args(flags)
                .arg(line)
                .env(vw_agent_tools::hook::ENV_ENDPOINT_FILE, &endpoint)
                .output()
                .unwrap();
            assert!(output.status.success(), "{shell:?}: {line}: {output:?}");
            assert_eq!(
                String::from_utf8(output.stdout).unwrap(),
                format!("kill-guard\n{}\n", endpoint.display()),
                "{shell:?}: {name}"
            );
            if flags.contains(&"-Command") {
                // A filesystem PSDrive exercises the Windows drive-root command token on this Linux host.
                let drive = std::path::PathBuf::from(format!("V:{}", binary.display()));
                let line = format!(
                    "New-PSDrive -Name V -PSProvider FileSystem -Root / | Out-Null; {}",
                    hook_command(&drive, &endpoint, "kill-guard", true)
                );
                let output = Command::new(shell)
                    .args(flags)
                    .arg(&line)
                    .env(vw_agent_tools::hook::ENV_ENDPOINT_FILE, &endpoint)
                    .output()
                    .unwrap();
                assert!(output.status.success(), "{name}: {output:?}");
                assert_eq!(
                    String::from_utf8(output.stdout).unwrap(),
                    format!("kill-guard\n{}\n", endpoint.display())
                );
            }
        }
    }
}

/// A person's own hooks in the app's hooks.toml (any type, here Vibe's `notification`) stay when the app writes
/// or removes its own block, so they keep running after a restart.
#[test]
fn with_block_keeps_a_person_notification_hook() {
    let own =
        "[[hooks]]\nname = \"ping\"\ntype = \"notification\"\ncommand = \"notify-send done\"\n";
    let block = super::hook_block(
        std::path::Path::new("/app/vw-hook"),
        std::path::Path::new("/data/hook-endpoint"),
    );
    let first = super::with_block(own, Some(&block));
    let restarted = super::with_block(&first, Some(&block));
    assert_eq!(first, restarted, "rewriting the block changes nothing else");
    for text in [&first, &restarted] {
        assert!(text.starts_with(own.trim_end()), "{text}");
        let parsed: toml::Table = text.parse().expect("valid TOML");
        let hooks = parsed["hooks"].as_array().unwrap();
        assert!(
            hooks.iter().any(|h| h["name"].as_str() == Some("ping")
                && h["type"].as_str() == Some("notification")),
            "{text}"
        );
    }
    let removed = super::with_block(&first, None);
    assert_eq!(removed.trim_end(), own.trim_end());
}

// PREDICT (refusal block): when vw-agent-tools.exe cannot start, the strict guard and the shell wrapper (both that
// program) fail before any code runs, so every tool result said only "exited with code 3221225781" and the agent named
// the wrong cause. The refusal block's command must print a deny with the plain reason in sh, Git Bash and PowerShell.

/// The one hook of a refusal block, parsed as Vibe reads `hooks.toml`.
fn refusal_hook(block: &str) -> toml::Table {
    let table: toml::Table = toml::from_str(block).unwrap();
    let hooks = table["hooks"].as_array().unwrap();
    assert_eq!(hooks.len(), 1, "{block}");
    hooks[0].as_table().unwrap().clone()
}

/// The shells a Windows hook line runs in (Git Bash, PowerShell) or a Unix one (sh), as available here.
fn hook_shells() -> Vec<(std::ffi::OsString, Vec<&'static str>)> {
    let mut shells: Vec<(std::ffi::OsString, Vec<&'static str>)> = Vec::new();
    if cfg!(windows) {
        shells.push((
            "powershell".into(),
            vec!["-NoLogo", "-NoProfile", "-NonInteractive", "-Command"],
        ));
        let git_bash = Path::new(r"C:\Program Files\Git\bin\bash.exe");
        if git_bash.is_file() {
            shells.push((git_bash.into(), vec!["-c"]));
        }
    } else {
        shells.push(("/bin/sh".into(), vec!["-c"]));
        shells.push(("/bin/bash".into(), vec!["-c"]));
    }
    if let Some(pwsh) = std::env::var_os("VW_TEST_POWERSHELL") {
        shells.push((
            pwsh,
            vec!["-NoLogo", "-NoProfile", "-NonInteractive", "-Command"],
        ));
    }
    shells
}

#[test]
fn a_tools_program_that_cannot_start_refuses_every_tool_with_the_plain_reason() {
    let problem = "Windows could not load it: error 0xC0000135, a DLL it needs is missing";
    let reason = super::unavailable_reason(problem);
    assert_eq!(
        reason,
        "Not run: the Vibe Work tool program vw-agent-tools.exe cannot start on this computer (Windows could not load \
         it: error 0xC0000135, a DLL it needs is missing), so no command, file, process or browser tool can run in \
         this chat. Vibe Work is not installed completely; running the Vibe Work installer again repairs it, and a new \
         chat then has its tools."
            .replace(".exe", if cfg!(windows) { ".exe" } else { "" })
    );
    let hook = refusal_hook(&super::refusal_block(&reason));
    assert_eq!(hook["type"].as_str(), Some("pre_tool"));
    assert_eq!(hook["match"].as_str(), Some("*"));
    assert_eq!(hook["strict"].as_bool(), Some(true));
    // Interrupted sessions bound to the combined guard's name still resolve to this refusal.
    assert_eq!(
        hook["name"].as_str(),
        Some(vw_agent_tools::hook::Guard::Safety.hook_name())
    );
    let command = hook["command"].as_str().unwrap();
    // The agent sees exactly the reason, with the one character no shell line may hold replaced.
    let expected = reason.replace('\'', "?");
    for (shell, args) in hook_shells() {
        let output = std::process::Command::new(&shell)
            .args(&args)
            .arg(command)
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap();
        assert!(output.status.success(), "{shell:?}: {output:?}");
        let answer: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(answer["decision"], "deny", "{shell:?}");
        assert_eq!(answer["reason"], expected.as_str(), "{shell:?}");
    }
}

#[test]
fn a_refusal_reason_with_shell_characters_stays_one_plain_line() {
    let reason =
        "it could not be started (Zugriff verweigert: C:\\a b\\$x `y` 'z' ‘q’ \"w\"; & | > <)";
    let hook = refusal_hook(&super::refusal_block(reason));
    let command = hook["command"].as_str().unwrap();
    for (shell, args) in hook_shells() {
        let output = std::process::Command::new(&shell)
            .args(&args)
            .arg(command)
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap();
        let answer: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            answer["reason"],
            "it could not be started (Zugriff verweigert: C:/a b/$x `y` ?z? ?q? \"w\"; & | > <)",
            "{shell:?}"
        );
    }
}

#[test]
fn the_next_start_replaces_a_refusal_with_the_normal_guards() {
    let refused = super::with_block(
        "[[hooks]]\nname = \"mine\"\n",
        Some(&super::refusal_block("x")),
    );
    let normal = super::hook_block(
        Path::new("/opt/vw-agent-tools"),
        Path::new("/data/hook.json"),
    );
    let replaced = super::with_block(&refused, Some(&normal));
    assert!(
        replaced.starts_with("[[hooks]]\nname = \"mine\"\n"),
        "{replaced}"
    );
    assert!(!replaced.contains("decision"), "{replaced}");
    assert!(replaced.contains("kill-guard"), "{replaced}");
}
