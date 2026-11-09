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
        for (shell, flags) in &shells {
            let output = Command::new(shell)
                .args(flags)
                .arg(&line)
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
