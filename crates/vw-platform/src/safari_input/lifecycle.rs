//! Portable preparation ordering: native discovery can block; cancellation must fence later window activation.
//! The macOS worker supplies its AX discovery/activation and live operation check; Linux tests interrupt discovery.
pub(super) fn prepare<T>(
    discover: impl FnOnce() -> Result<T, String>,
    live: impl Fn() -> Result<(), String>,
    activate: impl FnOnce(&T, &dyn Fn() -> Result<(), String>) -> Result<(), String>,
) -> Result<T, String> {
    live()?;
    let window = discover()?;
    live()?;
    activate(&window, &live)?;
    live()?;
    Ok(window)
}
/// Accessibility gate before any Safari window lookup. Native input needs the app to be trusted for Accessibility.
/// An agent action only reads the trust state and never shows the system prompt: an untrusted app refuses with the
/// System Settings instruction. The one system prompt is shown when Safari pairing completes (`request_accessibility`).
pub(super) fn require_accessibility(trusted: impl FnOnce() -> bool) -> Result<(), String> {
    if trusted() {
        Ok(())
    } else {
        Err(NOT_TRUSTED.into())
    }
}
const NOT_TRUSTED: &str = "Vibe Work has no Accessibility permission, so native Safari input was refused. Ask the user to turn on Vibe Work in System Settings > Privacy & Security > Accessibility, then try again.";
pub(super) fn confirm(
    live: impl Fn() -> Result<(), String>,
    confirmation: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    live()?;
    confirmation()?;
    live()
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    // PREDICT (before the fix): the first refused agent action in an app run calls the macOS prompt and answers
    // "macOS now shows its Accessibility prompt", so an agent's key or click raises a system dialog.
    // After the fix: an agent action never prompts; it refuses with the System Settings instruction every time.
    // The one system prompt belongs to Safari pairing completion (vw-core-api `safari::observe`).
    #[test]
    fn untrusted_agent_action_refuses_without_the_system_prompt() {
        // The gate has no prompt input at all; it reads trust once per action and refuses each time.
        let trust_reads = Cell::new(0);
        for _ in 0..2 {
            let refusal = require_accessibility(|| {
                trust_reads.set(trust_reads.get() + 1);
                false
            })
            .expect_err("an untrusted app refuses native input");
            assert!(
                refusal.contains(
                    "turn on Vibe Work in System Settings > Privacy & Security > Accessibility"
                ),
                "the refusal tells the agent where the user turns the permission on: {refusal}"
            );
            assert!(
                !refusal.contains("prompt"),
                "no system prompt is announced: {refusal}"
            );
        }
        assert_eq!(
            trust_reads.get(),
            2,
            "every agent action rechecks the current permission"
        );
    }
    #[test]
    fn trusted_app_runs_native_input() {
        assert!(require_accessibility(|| true).is_ok());
    }
    #[test]
    fn stop_during_native_discovery_never_activates_a_window() {
        let stopped = Cell::new(false);
        let raised = Cell::new(false);
        let result = prepare(
            || {
                stopped.set(true);
                Ok(())
            },
            || {
                if stopped.get() {
                    Err("stopped".into())
                } else {
                    Ok(())
                }
            },
            |(), _| {
                raised.set(true);
                Ok(())
            },
        );
        assert!(result.is_err());
        assert!(
            !raised.get(),
            "no window activation after Stop during discovery"
        );
    }
    #[test]
    fn stop_during_native_focus_confirmation_refuses_input() {
        let stopped = Cell::new(false);
        let result = confirm(
            || {
                if stopped.get() {
                    Err("stopped".into())
                } else {
                    Ok(())
                }
            },
            || {
                stopped.set(true);
                Ok(())
            },
        );
        assert!(
            result.is_err(),
            "a cancelled focus confirmation cannot permit a native event"
        );
    }
}
