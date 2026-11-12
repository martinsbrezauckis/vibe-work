//! Document-scoped records of dispatched input with no verified receipt; called by the action loop.
//! Stores bounded fingerprints rather than field contents; a reread cannot erase an unresolved effect.
use super::{HashSet, Seen, Sessions};
use std::hash::{Hash, Hasher};

pub(crate) struct Key {
    mount: vw_preview::MountId,
    document: String,
    fingerprint: u64,
}

#[derive(Default)]
pub(super) struct Fence {
    binding: Option<(vw_preview::MountId, String)>,
    inputs: HashSet<u64>,
}

impl Key {
    pub(crate) fn new(seen: Option<&Seen>, action: &crate::Action) -> Option<Self> {
        let seen = seen?;
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        format!("{action:?}").hash(&mut hash);
        Some(Self {
            mount: seen.mount.clone(),
            document: seen.model.document.clone(),
            fingerprint: hash.finish(),
        })
    }
}

impl Sessions {
    pub(crate) fn unresolved_input(&self, session: &str, key: Option<&Key>) -> bool {
        let Some(key) = key else {
            return false;
        };
        self.lock().get(session).is_some_and(|session| {
            let fence = &session.unknown_input;
            fence
                .binding
                .as_ref()
                .is_some_and(|(mount, document)| *mount == key.mount && *document == key.document)
                && (fence.inputs.len() >= 256 || fence.inputs.contains(&key.fingerprint))
        })
    }
    pub(crate) fn remember_unknown_input(&self, session: &str, key: Option<Key>) {
        let Some(key) = key else {
            return;
        };
        let mut sessions = self.lock();
        let fence = &mut sessions.entry(session.into()).or_default().unknown_input;
        if fence
            .binding
            .as_ref()
            .is_none_or(|(mount, document)| *mount != key.mount || *document != key.document)
        {
            fence.inputs.clear();
            fence.binding = Some((key.mount, key.document));
        }
        if fence.inputs.len() < 256 {
            fence.inputs.insert(key.fingerprint);
        }
    }
}

impl crate::Browser {
    pub(crate) fn check_unknown_input(
        &self,
        session: &str,
        action: &crate::Action,
        actor: crate::Actor,
    ) -> Result<Option<Key>, crate::ActError> {
        let key = Key::new(self.current_seen(session).as_ref(), action);
        if actor == crate::Actor::Agent && self.sessions.unresolved_input(session, key.as_ref()) {
            return Err(crate::driver::refused(
                "An identical input has an unresolved outcome in this document. Read the page and inspect its actual state before choosing a different action; this input was not repeated.",
            ));
        }
        Ok(key)
    }
    pub(crate) fn finish_unknown_input(
        &self,
        session: &str,
        key: Option<Key>,
        result: Result<serde_json::Value, crate::ActError>,
        sent: bool,
    ) -> Result<serde_json::Value, crate::ActError> {
        let result = Self::finish_native_action(result, sent);
        if sent && result.is_err() {
            self.sessions.remember_unknown_input(session, key);
        }
        result
    }
}
