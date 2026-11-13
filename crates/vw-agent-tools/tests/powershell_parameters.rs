//! Windows PowerShell's command position in the Git and process guards: `powershell.exe` reads parameters (and the
//! values of those that take one) until the first plain word or unknown spelling, which starts the command.
//!
//! PREDICT: the guards read PowerShell's plain words as possible command starts, but only the first eight. A line
//! with eight or more parameter values before the command (`-w Hidden` nine times, then the command) hid the command
//! from both guards: `powershell.exe` runs it (checked on Windows PowerShell 5.1), the guards saw no credential read
//! and no process stop. Command text PowerShell takes from standard input (`-Command -`, a lone `-`, `-File -`) or a
//! server mode was not read at all and passed both guards without a question; nor was a call with no command at all,
//! which runs what is piped into it (`echo … | powershell -NoProfile`, checked on 5.1 and 7.6). PowerShell 7 also
//! takes `\u{2013}Command`, `--command` and `-CommandWithArgs`/`-cwa`, which the old reading did not know.
#![allow(clippy::unwrap_used)]
use std::path::Path;

use serde_json::{Value, json};
use vw_agent_tools::hook::Guard;
use vw_agent_tools::{gitparse, killparse};

/// Every value-taking parameter this reader handles, with a value `powershell.exe` accepts or at least consumes.
const VALUE_PARAMETERS: [(&str, &str); 22] = [
    ("-ConfigurationName", "Microsoft.PowerShell"),
    ("-config", "Microsoft.PowerShell"),
    ("-WindowStyle", "Hidden"),
    ("-w", "Hidden"),
    ("-WorkingDirectory", "C:\\"),
    ("-wo", "C:\\"),
    ("-wd", "C:\\"),
    ("-OutputFormat", "Text"),
    ("-o", "Text"),
    ("-of", "Text"),
    ("-InputFormat", "Text"),
    ("-i", "Text"),
    ("-if", "Text"),
    ("-ExecutionPolicy", "Bypass"),
    ("-ex", "Bypass"),
    ("-ep", "Bypass"),
    ("-EncodedArguments", "AAAA"),
    ("-ea", "AAAA"),
    ("-PSConsoleFile", "shell.psc1"),
    ("-p", "shell.psc1"),
    ("-Version", "5.1"),
    ("/v", "5.1"),
];

fn git_decision(command: &str) -> Value {
    let endpoint = Path::new("nonexistent-powershell-parameters-endpoint.json");
    let payload = json!({"tool_name":"file_system.bash", "tool_input":{"command":command}});
    Guard::Git
        .decide(&payload.to_string(), endpoint)
        .map_or(Value::Null, |text| serde_json::from_str(&text).unwrap())
}

fn padded(parameter: &str, value: &str, times: usize, command: &str) -> String {
    let values = format!(" {parameter} {value}").repeat(times);
    format!("powershell{values} \"{command}\"")
}

#[test]
fn eight_or_more_parameter_values_do_not_hide_the_command() {
    for times in [4, 8, 9, 20] {
        let line = padded("-w", "Hidden", times, "git credential fill");
        assert!(gitparse::scan(&line).credential_read(), "{line}");
        assert_eq!(git_decision(&line)["decision"], "deny", "{line}");
        let line = padded("-w", "Hidden", times, "Stop-Process -Id 4242 -Force");
        let scan = killparse::scan(&line);
        assert!(
            scan.targets
                .contains(&killparse::KillTarget::Pid { pid: 4242 }),
            "{line}: {scan:?}"
        );
    }
    let mixed = "powershell -NoProfile -NonInteractive -w Hidden -ep Bypass -if Text -of Text -wd C:\\ -nologo -sta \
                 -wd C:\\ -w Hidden -ep Bypass -WindowStyle Hidden \"git credential fill\"";
    assert!(gitparse::scan(mixed).credential_read(), "{mixed}");
    assert_eq!(git_decision(mixed)["decision"], "deny", "{mixed}");
}

#[test]
fn each_value_taking_parameter_is_skipped_with_its_value() {
    for (parameter, value) in VALUE_PARAMETERS {
        let line = padded(parameter, value, 9, "git credential fill");
        assert!(gitparse::scan(&line).credential_read(), "{line}");
        assert_eq!(git_decision(&line)["decision"], "deny", "{line}");
        let line = padded(parameter, value, 9, "taskkill /PID 31 /F");
        let scan = killparse::scan(&line);
        assert!(
            scan.targets
                .contains(&killparse::KillTarget::Pid { pid: 31 }),
            "{line}: {scan:?}"
        );
    }
}

#[test]
fn powershell_text_that_cannot_be_read_is_asked_about() {
    for command in [
        "powershell -NoProfile -Command -",
        "echo \"git credential fill\" | powershell -NoProfile -",
        "type stop.txt | powershell -File -",
        "powershell -w Hidden -w Hidden -w Hidden -w Hidden -w Hidden -w Hidden -w Hidden -w Hidden -w Hidden -c -",
        "powershell -NoProfile -s",
        "powershell -NamedPipeServerMode",
        "powershell -enc not*base64",
        "pwsh -NoProfile -Command -",
        "cmd /c powershell -Command -",
        "echo \"Stop-Process -Id 5\" | powershell -NoProfile",
        "echo \"git credential fill\" | powershell -NoProfile -NonInteractive -w Hidden",
        "type cmds.txt | pwsh -NoProfile",
        "pwsh -NoProfile -",
        "pwsh -NoProfile -zzz",
    ] {
        // Git guard: the call goes to the core, which holds it for the user; without a core it is refused.
        assert!(gitparse::scan(command).needs_core(), "{command}");
        assert_eq!(git_decision(command)["decision"], "deny", "{command}");
        // Process guard: a mention it could not read, which the core asks the user about.
        let scan = killparse::scan(command);
        assert!(scan.mentions && !scan.understood(), "{command}: {scan:?}");
    }
}

#[test]
fn readable_powershell_lines_stay_unasked() {
    for command in [
        "powershell -NoProfile -Command \"git status\"",
        "powershell -w Hidden -w Hidden -w Hidden -w Hidden -w Hidden -w Hidden -w Hidden -w Hidden -w Hidden \"Get-Date\"",
        "powershell -NoProfile -File build.ps1",
        "pwsh -File build.ps1 -Configuration Release",
        "powershell -? ",
    ] {
        assert!(!gitparse::scan(command).needs_core(), "{command}");
        assert!(git_decision(command).is_null(), "{command}");
        assert!(!killparse::scan(command).mentions, "{command}");
    }
}

#[test]
fn powershell_seven_command_spellings_are_read() {
    for command in [
        "pwsh \u{2013}Command \"git credential fill\"",
        "pwsh --command \"git credential fill\"",
        "pwsh -cwa \"git credential fill\"",
        "pwsh -NoProfile -CommandWithArgs \"git credential fill\" x",
        "pwsh -w Hidden -w Hidden -w Hidden -w Hidden -w Hidden -w Hidden -w Hidden -w Hidden -w Hidden -c \"git credential fill\"",
    ] {
        assert!(gitparse::scan(command).credential_read(), "{command}");
        assert_eq!(git_decision(command)["decision"], "deny", "{command}");
    }
    let stop = killparse::scan("pwsh -cwa \"Stop-Process -Id 77 -Force\"");
    assert!(
        stop.targets
            .contains(&killparse::KillTarget::Pid { pid: 77 }),
        "{stop:?}"
    );
}
