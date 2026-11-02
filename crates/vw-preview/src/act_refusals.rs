//! What the engine refused during an agent act, on the person's behalf: a file picker the page opened (never
//! shown; the agent attaches files with its upload action) and a full-screen request. The engine reports each one
//! here so the act's receipt can say so.
//! Depends on the preview state; called by the engine through `PreviewHooks::act_refused` and by the browser
//! driver around an act.

use crate::{PreviewService, hooks::Hooks};

/// One thing the engine refused during an agent act.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActRefusal {
    /// A file picker the page opened; it never showed.
    FilePicker,
    /// A full-screen request; it was refused.
    FullScreen,
}

impl ActRefusal {
    pub(crate) const COUNT: usize = 2;

    pub(crate) const fn index(self) -> usize {
        match self {
            Self::FilePicker => 0,
            Self::FullScreen => 1,
        }
    }
}

impl Hooks {
    pub(crate) fn note_act_refusal(&self, label: &str, kind: ActRefusal) {
        let mut state = self.0.lock();
        if let Some(session) = state.session_of(label).map(str::to_owned) {
            let count = &mut state.entry(&session).act_refusals[kind.index()];
            *count = count.saturating_add(1);
        }
    }
}

impl PreviewService {
    /// How often the engine refused each kind on this page so far (a mark to compare after an act).
    #[must_use]
    pub fn act_refusal_mark(&self, session_id: &str, kind: ActRefusal) -> u64 {
        self.inner
            .lock()
            .entries
            .get(session_id)
            .map_or(0, |e| e.act_refusals[kind.index()])
    }
}
