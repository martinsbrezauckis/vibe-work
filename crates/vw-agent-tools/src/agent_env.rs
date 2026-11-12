//! Git and forge credential environment for processes that can run agent commands.
//! Depends on: std. Called by the server, agent process, mediated run and terminal launchers.
//! This is a guardrail against ambient credentials, not isolation from same-user files or OS agents.

use std::ffi::OsString;

/// The config key that sends every push to a transport git does not have; the name is what the agent reads in git's
/// error ("Unable to find remote helper for 'push-is-the-users-step-in-vibe-work'").
pub const PUSH_REFUSED_KEY: &str = "url.push-is-the-users-step-in-vibe-work:.pushInsteadOf";

/// Git for Windows variable that replaces git's standard input with a file before git's own start-up code runs.
/// On Windows the server's environment sets it to `NUL` (`vw_supervisor` `build_env`); the agent's shells remove it
/// again (the Git Bash start-up file, the `bash.exe` and PowerShell wrappers), so a pipe the agent writes into git
/// still reaches git. Git removes it from its own environment before it starts any program.
pub const GIT_STDIN_VAR: &str = "GIT_REDIRECT_STDIN";

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
    env.retain(|(name, value)| {
        let Some(name) = name.to_str() else {
            return false;
        };
        let upper = name.to_ascii_uppercase();
        if matches!(upper.as_str(), "HTTP_PROXY" | "HTTPS_PROXY" | "ALL_PROXY")
            && value.to_string_lossy().contains('@')
        {
            return false;
        }
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
