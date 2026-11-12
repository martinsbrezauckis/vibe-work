//! Names of app-owned modal dialogs used to explain a temporary browser-action refusal.
//! The UI reports its modal; native callers hold a lifetime guard. Page dialogs are separate browser targets.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

#[derive(Default)]
pub(crate) struct Dialogs {
    inner: Mutex<State>,
}

#[derive(Default)]
struct State {
    ui: Option<String>,
    next: u64,
    native: BTreeMap<u64, String>,
}

/// Keeps a native app-modal name registered until its window closes or its request is canceled.
pub struct BrowserDialog {
    dialogs: Arc<Dialogs>,
    id: u64,
}

impl Drop for BrowserDialog {
    fn drop(&mut self) {
        self.dialogs
            .inner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .native
            .remove(&self.id);
    }
}

impl Dialogs {
    pub(crate) fn set_ui(&self, name: Option<String>) {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner).ui =
            name.map(|name| label(&name));
    }

    fn hold(self: &Arc<Self>, name: &str) -> BrowserDialog {
        let mut state = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        state.next += 1;
        let id = state.next;
        state.native.insert(id, label(name));
        BrowserDialog {
            dialogs: self.clone(),
            id,
        }
    }

    pub(crate) fn check(&self) -> Result<(), String> {
        let state = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(name) = state
            .native
            .last_key_value()
            .map(|(_, name)| name)
            .or(state.ui.as_ref())
        {
            return Err(format!(
                "APP_DIALOG_OPEN: The app dialog {} is open, so this browser action was not run. Ask the user to finish or close that dialog, then retry the browser action.",
                vw_gate::dialog_quote(name)
            ));
        }
        Ok(())
    }
}

fn label(name: &str) -> String {
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() {
        "Vibe Work".into()
    } else {
        name.chars().take(120).collect()
    }
}

impl crate::Core {
    /// Holds the name of an app-modal native dialog; dropping the guard clears only that dialog.
    #[must_use]
    pub fn browser_dialog(&self, name: &str) -> BrowserDialog {
        self.shared.browser_dialogs.hold(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_dialogs_clear_only_their_own_name() {
        let dialogs = Arc::new(Dialogs::default());
        dialogs.set_ui(Some("Settings dialog".into()));
        let first = dialogs.hold("Choose documents");
        let second = dialogs.hold("Connect Chrome?");
        drop(first);
        assert!(dialogs.check().unwrap_err().contains("Connect Chrome?"));
        drop(second);
        assert!(dialogs.check().unwrap_err().contains("Settings dialog"));
        dialogs.set_ui(None);
        assert!(dialogs.check().is_ok());
    }
}
