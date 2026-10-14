//! Durable browser delegation receipts for later session review.
//! Depends on the typed browser receipt and the platform's atomic private writer; the core's browser delegation
//! wiring records completed attempts and hydrates loaded session state. Corrupt files are never overwritten.

use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::sync::Mutex;

use serde_json::Value;
use vw_browser::delegation::DelegationReceipt;

struct Contents {
    by_session: BTreeMap<String, Vec<DelegationReceipt>>,
    load_error: Option<String>,
}

/// A metadata-only receipt store backed by one private JSON file.
pub(crate) struct ReceiptStore {
    path: PathBuf,
    contents: Mutex<Contents>,
}

impl ReceiptStore {
    /// Loads all receipts. A missing file starts empty; a corrupt or unreadable file is retained and blocks writes.
    pub(crate) fn load(path: PathBuf) -> Self {
        let (by_session, load_error) = match std::fs::read(&path) {
            Ok(bytes) => match decode(&bytes) {
                Ok(rows)
                    if rows.iter().all(|(session, receipts)| {
                        receipts
                            .iter()
                            .all(|receipt| &receipt.session_id == session)
                    }) =>
                {
                    (rows, None)
                }
                Ok(_) => (
                    BTreeMap::new(),
                    Some("receipt session identifiers do not match".to_owned()),
                ),
                Err(error) => (
                    BTreeMap::new(),
                    Some(format!("invalid receipt JSON: {error}")),
                ),
            },
            Err(error) if error.kind() == ErrorKind::NotFound => (BTreeMap::new(), None),
            Err(error) => (
                BTreeMap::new(),
                Some(format!("cannot read receipt file: {error}")),
            ),
        };
        if let Some(error) = &load_error {
            tracing::warn!(%error, "browser delegation receipts unavailable; existing file retained");
        }
        Self {
            path,
            contents: Mutex::new(Contents {
                by_session,
                load_error,
            }),
        }
    }

    /// Persists one completed authorized attempt before making it visible in memory.
    pub(crate) fn record(&self, receipt: DelegationReceipt) -> Result<(), String> {
        let mut contents = self
            .contents
            .lock()
            .map_err(|_| "receipt store lock poisoned".to_owned())?;
        if let Some(error) = &contents.load_error {
            return Err(format!(
                "receipt store cannot write over its unreadable file: {error}"
            ));
        }
        let mut next = contents.by_session.clone();
        next.entry(receipt.session_id.clone())
            .or_default()
            .push(receipt);
        self.write(&next)?;
        contents.by_session = next;
        Ok(())
    }

    /// Removes one deleted session's receipt history and leaves other sessions untouched.
    pub(crate) fn forget(&self, session_id: &str) -> Result<(), String> {
        let mut contents = self
            .contents
            .lock()
            .map_err(|_| "receipt store lock poisoned".to_owned())?;
        if let Some(error) = &contents.load_error {
            return Err(format!(
                "receipt store cannot write over its unreadable file: {error}"
            ));
        }
        if !contents.by_session.contains_key(session_id) {
            return Ok(());
        }
        let mut next = contents.by_session.clone();
        next.remove(session_id);
        self.write(&next)?;
        contents.by_session = next;
        Ok(())
    }

    /// Returns the retained receipts, grouped by session id and ordered within each session by recording time.
    pub(crate) fn all(&self) -> Vec<DelegationReceipt> {
        self.contents
            .lock()
            .map(|contents| contents.by_session.values().flatten().cloned().collect())
            .unwrap_or_default()
    }

    fn write(&self, receipts: &BTreeMap<String, Vec<DelegationReceipt>>) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(receipts)
            .map_err(|error| format!("receipt serialization failed: {error}"))?;
        vw_platform::replace_private(&self.path, &bytes)
            .map_err(|error| format!("receipt write failed: {error}"))
    }
}

/// Retain tab-close receipts from an older mixed file. Obsolete consent and robot acts carry
/// enum variants the current contract no longer knows, so discard only those rows.
fn decode(bytes: &[u8]) -> Result<BTreeMap<String, Vec<DelegationReceipt>>, serde_json::Error> {
    let old: BTreeMap<String, Vec<Value>> = serde_json::from_slice(bytes)?;
    old.into_iter()
        .map(|(session, rows)| {
            rows.into_iter()
                .filter(|row| {
                    !matches!(
                        row.get("act").and_then(Value::as_str),
                        Some(
                            "robotCheckbox"
                                | "rejectCookies"
                                | "acceptCookies"
                                | "customCookies"
                                | "agreement"
                        )
                    )
                })
                .map(serde_json::from_value)
                .collect::<Result<Vec<_>, _>>()
                .map(|rows| (session, rows))
        })
        .collect()
}
