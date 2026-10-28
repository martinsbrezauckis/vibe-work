//! The vault's own settings file, `vault-settings.json` next to the key settings: the "Lock the vault when Vibe Work
//! quits" switch (`lockVaultOnQuit`), and `lockedUntilPassword` while an updated install waits for its first
//! password unlock. No secret is ever written here.
//!
//! The switch is off by default: the vault keeps its last state. Installs from before the switch had a "remember
//! this computer" choice instead; the first start that finds no switch takes it from that choice, once
//! ([`VaultSettings::at_start`]): a vault that was not remembered gets the switch on (its user still types the
//! password at every start), a remembered one gets it off and starts locked until its password is typed. Lock
//! did not delete the kept key then and left nothing on disk, so a remembered vault its user locked looks the same
//! as one left unlocked; asking for the password once never unlocks a vault the user locked. A separate file
//! because the key settings file's presence marks an install from before vault selection.
//!
//! Depends on: `serde_json`. Called by: `service`.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};
use tracing::warn;

use crate::error::KeyError;

/// The switch's field name.
const FIELD: &str = "lockVaultOnQuit";
/// Set while the vault waits for its password before a kept key may open it.
const UNTIL_PASSWORD: &str = "lockedUntilPassword";

/// What a start reads from the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AtStart {
    /// "Lock the vault when Vibe Work quits".
    pub lock_on_quit: bool,
    /// The vault stays locked until a password unlock, whatever key is kept.
    pub locked_until_password: bool,
}

/// The settings file.
pub struct VaultSettings {
    path: PathBuf,
}

impl VaultSettings {
    /// The file next to the key settings file `key_settings`.
    #[must_use]
    pub fn next_to(key_settings: &Path) -> Self {
        Self {
            path: key_settings.with_file_name("vault-settings.json"),
        }
    }

    /// The file's fields; `None` when it is missing or not a JSON object.
    fn read(&self) -> Option<Map<String, Value>> {
        let raw = std::fs::read(&self.path).ok()?;
        if let Ok(Value::Object(fields)) = serde_json::from_slice::<Value>(&raw) {
            return Some(fields);
        }
        warn!(path = %self.path.display(), "vault settings: unreadable file");
        None
    }

    /// The switch and the wait for the password, saving them first when this install has no switch yet.
    /// `remembered`: whether the vault had a kept key before this start (`None`: no vault yet). Only the first call
    /// that finds no saved switch writes; later calls return what was saved. A file that cannot be written leaves
    /// the values from the old choice for this run.
    pub fn at_start(&self, remembered: Option<bool>) -> AtStart {
        if let Some(fields) = self.read()
            && let Some(lock_on_quit) = fields.get(FIELD).and_then(Value::as_bool)
        {
            return AtStart {
                lock_on_quit,
                locked_until_password: fields.get(UNTIL_PASSWORD).and_then(Value::as_bool)
                    == Some(true),
            };
        }
        let at_start = AtStart {
            lock_on_quit: remembered == Some(false),
            locked_until_password: remembered == Some(true),
        };
        let saved = self.write(|fields| {
            fields.insert(FIELD.to_owned(), Value::Bool(at_start.lock_on_quit));
            if at_start.locked_until_password {
                fields.insert(UNTIL_PASSWORD.to_owned(), Value::Bool(true));
            }
        });
        if let Err(error) = saved {
            warn!(%error, "vault settings: the switch from the old choice was not saved");
        }
        at_start
    }

    /// Saves the switch (atomic replace; other fields are kept).
    ///
    /// # Errors
    /// [`KeyError::Settings`].
    pub fn set_lock_on_quit(&self, on: bool) -> Result<(), KeyError> {
        self.write(|fields| {
            fields.insert(FIELD.to_owned(), Value::Bool(on));
        })
    }

    /// Ends the wait for the password (after a password unlock).
    ///
    /// # Errors
    /// [`KeyError::Settings`].
    pub fn end_wait_for_password(&self) -> Result<(), KeyError> {
        self.write(|fields| {
            fields.remove(UNTIL_PASSWORD);
        })
    }

    /// Changes the fields and replaces the file atomically.
    fn write(&self, change: impl FnOnce(&mut Map<String, Value>)) -> Result<(), KeyError> {
        let mut fields = self.read().unwrap_or_default();
        change(&mut fields);
        let body = Value::Object(fields).to_string();
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| KeyError::Settings(e.to_string()))?;
        }
        let tmp = self.path.with_extension("tmp");
        std::fs::write(&tmp, body)
            .and_then(|()| std::fs::rename(&tmp, &self.path))
            .map_err(|e| KeyError::Settings(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> VaultSettings {
        let dir =
            std::env::temp_dir().join(format!("vw-vault-settings-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        VaultSettings::next_to(&dir.join("keys.json"))
    }

    fn at_start(lock_on_quit: bool, locked_until_password: bool) -> AtStart {
        AtStart {
            lock_on_quit,
            locked_until_password,
        }
    }

    #[test]
    fn migrates_once_from_the_old_choice() {
        let off = scratch("off");
        assert_eq!(
            off.at_start(Some(false)),
            at_start(true, false),
            "not remembered: switch on"
        );
        assert_eq!(
            off.at_start(Some(true)),
            at_start(true, false),
            "a second start keeps what was saved"
        );
        let on = scratch("on");
        assert_eq!(
            on.at_start(Some(true)),
            at_start(false, true),
            "remembered: switch off, locked until the password"
        );
        assert_eq!(on.at_start(Some(true)), at_start(false, true), "saved");
        on.end_wait_for_password().unwrap();
        assert_eq!(on.at_start(Some(true)), at_start(false, false));
        let fresh = scratch("fresh");
        assert_eq!(fresh.at_start(None), at_start(false, false), "no vault yet");
        let raw: Value = serde_json::from_slice(&std::fs::read(&fresh.path).unwrap()).unwrap();
        assert_eq!(raw, serde_json::json!({"lockVaultOnQuit": false}), "saved");
    }

    #[test]
    fn a_later_choice_wins_and_other_fields_stay() {
        let settings = scratch("later");
        std::fs::write(&settings.path, r#"{"other":1}"#).unwrap();
        assert!(settings.at_start(Some(false)).lock_on_quit);
        settings.set_lock_on_quit(false).unwrap();
        assert!(!settings.at_start(Some(false)).lock_on_quit);
        let raw: Value = serde_json::from_slice(&std::fs::read(&settings.path).unwrap()).unwrap();
        assert_eq!(raw["other"], 1);
    }
}
