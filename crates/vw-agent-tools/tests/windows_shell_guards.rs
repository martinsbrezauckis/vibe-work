//! Windows shells run their command text: `cmd //c` (Git Bash spelling), `cmd /c|/k`, `pwsh|powershell -Command` (and its
//! abbreviations). The Git credential guard and the process guard read that text like a `bash -c` payload.
use std::path::Path;

use serde_json::{Value, json};
use vw_agent_tools::hook::Guard;
use vw_agent_tools::{gitparse, killparse};

#[test]
fn windows_shell_wrapped_credential_reads_are_refused() {
    let endpoint = Path::new("nonexistent-credential-endpoint.json");
    for command in [
        "cmd //c \"(echo protocol=https& echo host=example.test& echo.) | git credential fill\"",
        "cmd /C \"git credential fill\"",
        "cmd.exe /c git credential fill",
        "pwsh -NoProfile -Command \"'protocol=https','host=example.test','' | git credential fill\"",
        "powershell -c \"git credential fill\"",
        "powershell.exe -NoProfile -com 'gh auth token'",
        "pwsh /Command \"git credential-manager get\"",
        // cmd reads `/c` with the text attached and drops `^` escapes.
        "cmd /cgit credential fill",
        "cmd /d /c \"g^it cred^ential fill\"",
        // Windows PowerShell runs its first plain argument as a command.
        "powershell -NoProfile -ExecutionPolicy Bypass \"git credential fill\"",
        // -EncodedCommand: base64 of UTF-16LE "git credential fill".
        "powershell -enc ZwBpAHQAIABjAHIAZQBkAGUAbgB0AGkAYQBsACAAZgBpAGwAbAA=",
        // A parameter value before -Command must not hide it; nor a short, unknown parameter before the positional text.
        "pwsh -wi Hidden -c \"git credential fill\"",
        "powershell -wi Hidden \"git credential fill\"",
        "cmd x /c git credential fill",
        // A positional command before a harmless -Command is still read.
        "powershell \"git credential fill;\" -c x",
    ] {
        assert!(
            gitparse::scan(command).credential_read(),
            "not seen as a credential read: {command}"
        );
        let payload = json!({"tool_name":"file_system.bash", "tool_input":{"command":command}});
        let answer: Value = serde_json::from_str(
            &Guard::Git
                .decide(&payload.to_string(), endpoint)
                .expect(command),
        )
        .unwrap();
        assert_eq!(answer["decision"], "deny", "{command}: {answer}");
    }
    for command in [
        "cmd //c \"echo git credential fill\"",
        "pwsh -NoProfile -Command \"Write-Output 'git credential fill'\"",
        "cmd /c dir",
        "pwsh -Command \"git status\"",
    ] {
        assert!(
            !gitparse::scan(command).credential_read(),
            "text or ordinary command read as a credential read: {command}"
        );
    }
}

#[test]
fn windows_shell_wrapped_process_stops_are_seen() {
    for (command, pid) in [
        ("cmd //c \"taskkill /PID 25612 /F\"", 25612),
        ("cmd /c taskkill /F /PID 4242", 4242),
        (
            "pwsh -NoProfile -Command \"Stop-Process -Id 14800 -Force\"",
            14800,
        ),
        ("powershell -c \"Stop-Process -Id 77 -Force\"", 77),
        ("cmd /ctaskkill /PID 31 /F", 31),
        ("powershell \"Stop-Process -Id 32 -Force\"", 32),
    ] {
        let scan = killparse::scan(command);
        assert!(scan.mentions, "process stop not seen: {command}");
        assert!(
            scan.targets.contains(&killparse::KillTarget::Pid { pid }),
            "{command}: {scan:?}"
        );
    }
    for command in [
        "cmd //c \"echo taskkill /PID 1 /F\"",
        "pwsh -Command \"Write-Output 'Stop-Process -Id 1'\"",
        "cmd /c dir",
        // pwsh's first plain argument is a script file, not command text.
        "pwsh script.ps1",
    ] {
        assert!(
            !killparse::scan(command).mentions,
            "text read as a process stop: {command}"
        );
    }
}

#[test]
fn shells_nested_too_deep_ask_instead_of_passing() {
    let scan = killparse::scan("cmd /c cmd /c cmd /c cmd /c taskkill /PID 9 /F");
    assert!(scan.mentions);
    assert!(!scan.understood(), "{scan:?}");
}
