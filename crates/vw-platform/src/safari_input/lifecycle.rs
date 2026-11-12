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
