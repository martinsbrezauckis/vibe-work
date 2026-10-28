//! Git and forge credential environment for processes that can run agent commands.
//! Depends on: std. Called by the server, agent process, mediated run and terminal launchers.
//! This is a guardrail against ambient credentials, not isolation from same-user files or OS agents.

use std::ffi::OsString;

/// The config key that sends every push to a transport git does not have; the name is what the agent reads in git's
/// error ("Unable to find remote helper for 'push-is-the-users-step-in-vibe-work'").
pub const PUSH_REFUSED_KEY: &str = "url.push-is-the-users-step-in-vibe-work:.pushInsteadOf";

/// Remove inherited git credential channels, pin Git's credential prompting off and turn every push address into one
/// git cannot reach. Call this after all other environment additions, immediately before a child is started.
///
/// GitHub reads use an app-owned broker through a credential-free PATH shim. Direct gh invocations
/// remain signed out: `GH_CONFIG_DIR` is the null device, so they cannot load or persist a login.
/// Neither the broker endpoint nor its shim needs a token in this environment.
///
/// The push rule adds a second layer under the push guard hook, for a
/// `git push` that reaches git anyway (a remote that needs no credentials, such as a local folder, would otherwise
/// receive it). `url.<base>.pushInsteadOf` with an EMPTY value
/// matches every address, also one typed on the command line, and rewrites only pushes (fetch and pull keep their
/// address), so a push from any process with this environment fails inside git, hook or not. The user's own Push
/// runs through vw-changes' hardened runner, whose environment is rebuilt from an allowlist without these keys.
/// This rule guards against an ordinary accidental push; git ignores it
/// for a remote with an explicit `pushurl`, a longer `pushInsteadOf` prefix in user or repository config wins, and
/// a command can override it (`-c`, a cleared environment). Plain `insteadOf` rewrites lose to it (push rewrites
/// are applied first). Blocking transports would also break fetch; a pre-push hook path would replace the
/// repository's own hooks for the agent's commits.
pub fn isolate_credentials(env: &mut Vec<(OsString, OsString)>) {
    env.retain(|(name, _)| {
        let Some(name) = name.to_str() else {
            return false;
        };
        let upper = name.to_ascii_uppercase();
        !upper.starts_with("GIT_CONFIG_")
            && !upper.starts_with("GCM_")
            && !matches!(
                upper.as_str(),
                "GH_TOKEN"
                    | "GITHUB_TOKEN"
                    | "GH_ENTERPRISE_TOKEN"
                    | "GITHUB_ENTERPRISE_TOKEN"
                    | "GH_CONFIG_DIR"
                    | "GIT_ASKPASS"
                    | "GIT_SSH"
                    | "GIT_SSH_COMMAND"
                    | "SSH_ASKPASS"
                    | "SSH_AUTH_SOCK"
                    | "GIT_TERMINAL_PROMPT"
                    | "GIT_CONFIG_PARAMETERS"
            )
    });
    // An empty credential.helper resets helper chains from system, global and repository config.
    // The environment config has higher precedence than on-disk config; the empty askPass
    // and terminal prompt switch prevent a GUI or terminal fallback.
    for (name, value) in [
        ("GIT_CONFIG_COUNT", "3"),
        ("GIT_CONFIG_KEY_0", "credential.helper"),
        ("GIT_CONFIG_VALUE_0", ""),
        ("GIT_CONFIG_KEY_1", "core.askPass"),
        ("GIT_CONFIG_VALUE_1", ""),
        ("GIT_CONFIG_KEY_2", PUSH_REFUSED_KEY),
        ("GIT_CONFIG_VALUE_2", ""),
        ("GIT_CONFIG_NOSYSTEM", "1"),
        ("GIT_TERMINAL_PROMPT", "0"),
        ("GCM_INTERACTIVE", "never"),
    ] {
        env.push((OsString::from(name), OsString::from(value)));
    }
    env.push((
        OsString::from("GH_CONFIG_DIR"),
        OsString::from(if cfg!(windows) { "NUL" } else { "/dev/null" }),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;
    use std::process::{Command, Stdio};

    #[test]
    fn removes_ambient_channels_and_replaces_stale_git_config() {
        let mut env = [
            ("PATH", "/usr/bin"),
            ("GH_TOKEN", "synthetic-token"),
            ("GITHUB_TOKEN", "synthetic-token"),
            ("GH_CONFIG_DIR", "/synthetic/auth"),
            ("SSH_AUTH_SOCK", "/synthetic/agent.sock"),
            ("GIT_ASKPASS", "/synthetic/askpass"),
            ("GIT_CONFIG_COUNT", "1"),
            ("GIT_CONFIG_KEY_0", "credential.helper"),
            ("GIT_CONFIG_VALUE_0", "!synthetic-helper"),
            ("GIT_CONFIG_KEY_19", "http.extraHeader"),
            ("GIT_CONFIG_VALUE_19", "synthetic-header"),
            ("git_config_global", "/synthetic/global"),
            ("gcm_interactive", "always"),
            ("gh_token", "synthetic-token"),
        ]
        .into_iter()
        .map(|(name, value)| (OsString::from(name), OsString::from(value)))
        .collect();
        isolate_credentials(&mut env);
        let find = |name: &str| {
            env.iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.to_string_lossy().into_owned())
        };
        assert_eq!(find("PATH").as_deref(), Some("/usr/bin"));
        assert_eq!(find("GIT_CONFIG_COUNT").as_deref(), Some("3"));
        assert_eq!(find("GIT_CONFIG_KEY_2").as_deref(), Some(PUSH_REFUSED_KEY));
        assert_eq!(find("GIT_CONFIG_VALUE_2").as_deref(), Some(""));
        assert_eq!(
            find("GIT_CONFIG_KEY_0").as_deref(),
            Some("credential.helper")
        );
        assert_eq!(find("GIT_CONFIG_VALUE_0").as_deref(), Some(""));
        assert_eq!(find("GIT_CONFIG_KEY_1").as_deref(), Some("core.askPass"));
        assert_eq!(find("GIT_CONFIG_VALUE_1").as_deref(), Some(""));
        assert_eq!(find("GIT_CONFIG_NOSYSTEM").as_deref(), Some("1"));
        assert_eq!(find("GIT_TERMINAL_PROMPT").as_deref(), Some("0"));
        assert_eq!(find("GCM_INTERACTIVE").as_deref(), Some("never"));
        // gh gets Vibe Work's own empty config folder, never the user's.
        assert_ne!(find("GH_CONFIG_DIR").as_deref(), Some("/synthetic/auth"));
        for name in [
            "GH_TOKEN",
            "GITHUB_TOKEN",
            "SSH_AUTH_SOCK",
            "GIT_ASKPASS",
            "GIT_CONFIG_KEY_19",
            "GIT_CONFIG_VALUE_19",
            "git_config_global",
            "gcm_interactive",
            "gh_token",
        ] {
            assert!(find(name).is_none(), "{name} survived");
        }
    }

    #[test]
    fn git_does_not_call_a_synthetic_credential_helper() {
        let root = std::env::temp_dir().join(format!("vw-agent-env-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        // The env-cleared child must not discover the checkout (or its scratch boundary) above this fixture.
        assert!(
            Command::new("git")
                .args(["init", "-q"])
                .arg(&root)
                .status()
                .unwrap()
                .success()
        );
        let store = root.join("synthetic-credentials");
        std::fs::write(&store, "https://synthetic:synthetic@example.invalid\n").unwrap();
        let store_path = store.to_string_lossy().replace('\\', "/");
        let mut agent = vec![
            (OsString::from("PATH"), std::env::var_os("PATH").unwrap()),
            (OsString::from("HOME"), root.as_os_str().to_owned()),
            (OsString::from("USERPROFILE"), root.as_os_str().to_owned()),
            (OsString::from("GIT_CONFIG_NOSYSTEM"), OsString::from("1")),
            (OsString::from("GIT_CONFIG_COUNT"), OsString::from("1")),
            (
                OsString::from("GIT_CONFIG_KEY_0"),
                OsString::from("credential.helper"),
            ),
            (
                OsString::from("GIT_CONFIG_VALUE_0"),
                OsString::from(format!("store --file=\"{store_path}\"")),
            ),
        ];
        let mut command = Command::new("git");
        command
            .current_dir(&root)
            .args(["credential", "fill"])
            .env_clear()
            .envs(agent.clone())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = command.spawn().unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"protocol=https\nhost=example.invalid\n\n")
            .unwrap();
        let user = child.wait_with_output().unwrap();
        assert!(user.status.success());
        assert!(String::from_utf8_lossy(&user.stdout).contains("password=synthetic"));

        isolate_credentials(&mut agent);
        let mut command = Command::new("git");
        command
            .current_dir(&root)
            .args(["credential", "fill"])
            .env_clear()
            .envs(agent)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = command.spawn().unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"protocol=https\nhost=example.invalid\n\n")
            .unwrap();
        let isolated = child.wait_with_output().unwrap();
        assert!(!isolated.status.success());
        assert_eq!(isolated.stdout, [] as [u8; 0]);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Nothing gh writes with the agent's environment survives into the next
    /// agent process, and the user's own gh config is never read: a `gh config set` in one process (the same path a
    /// `gh auth login` takes) is not seen by a second one, and a synthetic user config with `git_protocol: ssh` is not
    /// used. Falsifier: a config folder shared by both processes keeps the value (the second process reads "ssh").
    /// Offline. Skips (and says so) without gh.
    #[test]
    fn gh_keeps_nothing_between_agent_processes_and_ignores_the_users_config() {
        if Command::new("gh").arg("--version").output().is_err() {
            eprintln!("SKIPPED: gh is not installed on this machine");
            return;
        }
        let root = std::env::temp_dir().join(format!("vw-agent-gh-{}", std::process::id()));
        let config = root.join("gh-config");
        std::fs::create_dir_all(&config).unwrap();
        std::fs::write(
            config.join("config.yml"),
            "git_protocol: ssh\nversion: \"1\"\n",
        )
        .unwrap();
        let mut env = vec![
            (OsString::from("PATH"), std::env::var_os("PATH").unwrap()),
            (OsString::from("HOME"), root.as_os_str().to_owned()),
            (
                OsString::from("GH_CONFIG_DIR"),
                config.as_os_str().to_owned(),
            ),
            (
                OsString::from("GH_TOKEN"),
                OsString::from("synthetic-token"),
            ),
        ];
        isolate_credentials(&mut env);
        let run = |args: &[&str]| {
            Command::new("gh")
                .args(args)
                .env_clear()
                .envs(env.clone())
                .stdin(Stdio::null())
                .output()
                .unwrap()
        };
        let set = run(&["config", "set", "git_protocol", "ssh"]);
        let later = run(&["config", "get", "git_protocol"]);
        assert_ne!(
            String::from_utf8_lossy(&later.stdout).trim(),
            "ssh",
            "a value written by one agent process (set: {:?}) reached the next, or the user's config was used",
            set.status
        );
        assert!(
            !run(&["auth", "status"]).status.success(),
            "gh reports a sign-in"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
