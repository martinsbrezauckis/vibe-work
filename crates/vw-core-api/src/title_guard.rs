//! Chat titles never carry a vault value. Vibe titles a chat from its first message, which can hold a value the
//! person typed there, and shows the first message as the chat's preview until then. Every title and preview that
//! comes from Vibe (forwarded replies, notifications, the core's own reads and listings) passes [`withhold_in`]
//! before the core keeps or forwards it, so the chat list, the open chat's header, the bell and every other notice
//! that names a chat, the search index and the Debug API all get the same withheld text.
//!
//! Depends on: the key service (the vault's values), `secret_scan_cache::withhold_parts` (the rule the session-log
//! scan's findings use: every run of six or more characters that also occurs in a value), and
//! `secret_scan_worker::filter_inputs` (the same values the scan compares: long and strong enough to be told from
//! ordinary words). Called by: `dispatch` (every forwarded reply), `pump` (every notification, resync reads), the
//! core's own session reads and listings; `core` (the first read at start, and the owned [`watch`] task).
//! Invariants:
//! - Values are known once the vault was open in this run, and kept while it is locked (as the Debug API's
//!   registry keeps them, `vw-secrets`), so locking never brings a withheld value back. Before the vault was ever
//!   open in this run nothing can be compared, and titles pass unchanged.
//! - Only text changes: the string `title` and `preview` of any object, and the string `value` of a patch operation
//!   whose `path` ends in `/title` or `/preview`. Ids, structure and every other field stay as Vibe sent them.
//! - Values are never logged, returned or written anywhere.

use std::sync::{Arc, PoisonError, RwLock, Weak};

use serde_json::Value;
use tokio::sync::broadcast;
use vw_keys::VaultScanSecret;

use crate::core::Shared;
use crate::event::CoreEvent;
use crate::secret_scan_cache::withhold_parts;

/// The vault values titles are compared with (see the module invariants).
#[derive(Default)]
pub(crate) struct TitleGuard {
    values: RwLock<Vec<VaultScanSecret>>,
}

impl TitleGuard {
    /// `text` with the parts of a vault value withheld, or `None` when nothing is withheld.
    fn withheld(&self, text: &str) -> Option<String> {
        let values = self.values.read().unwrap_or_else(PoisonError::into_inner);
        if values.is_empty() {
            return None;
        }
        let exposed: Vec<&str> = values.iter().map(|v| v.value.expose()).collect();
        let out = withhold_parts(text, &exposed);
        (out != text).then_some(out)
    }
}

/// `text` with vault-value parts withheld by the same rule as titles (an edit preview's lines use it).
pub(crate) fn withhold_text(shared: &Shared, text: &str) -> String {
    shared
        .titles
        .withheld(text)
        .unwrap_or_else(|| text.to_owned())
}

/// Withholds vault-value parts in every chat title and preview inside `value` (module invariants).
pub(crate) fn withhold_in(shared: &Shared, value: &mut Value) {
    let guard = &shared.titles;
    if guard
        .values
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .is_empty()
    {
        return;
    }
    walk(guard, value);
}

fn walk(guard: &TitleGuard, value: &mut Value) {
    match value {
        Value::Object(map) => {
            let patches_text = map
                .get("path")
                .and_then(Value::as_str)
                .is_some_and(|path| path.ends_with("/title") || path.ends_with("/preview"));
            for (key, item) in map.iter_mut() {
                let text = key == "title" || key == "preview" || (patches_text && key == "value");
                match item {
                    Value::String(found) if text => {
                        if let Some(withheld) = guard.withheld(found) {
                            *found = withheld;
                        }
                    }
                    _ => walk(guard, item),
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|item| walk(guard, item)),
        _ => {}
    }
}

/// Reads the vault's values for [`withhold_in`]. A locked or missing vault keeps the values already known.
/// Call from a blocking thread (the vault is read).
fn refresh_now(shared: &Shared) {
    let Some(keys) = &shared.config.keys else {
        return;
    };
    let Ok(secrets) = keys.scan_secrets() else {
        return;
    };
    let (secrets, _) = crate::secret_scan_worker::filter_inputs(secrets);
    let mut values = shared
        .titles
        .values
        .write()
        .unwrap_or_else(PoisonError::into_inner);
    // Values removed from the vault stay: a title that held one is still withheld (nothing is brought back).
    for secret in secrets {
        if !values
            .iter()
            .any(|known| known.value.expose() == secret.value.expose())
        {
            values.push(secret);
        }
    }
}

/// Reads the vault's values now and waits for it (core start, before any chat is listed or loaded).
pub(crate) async fn refresh(shared: &Arc<Shared>) {
    let shared = shared.clone();
    let _ = tokio::task::spawn_blocking(move || refresh_now(&shared)).await;
}

/// The core's owned task: reads the vault's values again whenever the vault opens or locks (`keyStatusChanged`)
/// or its items change (`secretsChanged`), until the core goes away. A missed event (lag) reads them too.
pub(crate) async fn watch(shared: Weak<Shared>, mut events: broadcast::Receiver<CoreEvent>) {
    loop {
        match events.recv().await {
            Ok(CoreEvent::KeyStatusChanged {} | CoreEvent::SecretsChanged {})
            | Err(broadcast::error::RecvError::Lagged(_)) => {}
            Ok(_) => continue,
            Err(broadcast::error::RecvError::Closed) => return,
        }
        let Some(shared) = shared.upgrade() else {
            return;
        };
        refresh(&shared).await;
    }
}
