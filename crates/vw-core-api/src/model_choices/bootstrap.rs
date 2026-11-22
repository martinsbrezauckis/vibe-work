//! Keeps Vibe 3.1.4's built-in Medium alias available after its fetched catalog replaces defaults.
//! Called before the server starts for saved picks and after a person picks that built-in alias.

use std::path::Path;

use crate::core::Shared;
use crate::error::{CoreError, ErrorCode};

use super::{Choice, Records};

const ALIAS: &str = "mistral-medium-3.5";
const NAME: &str = "mistral-vibe-cli-latest";
const MARKER: &str = "built-in-medium-choice-v1";
const MODEL: &str = r#"

# Vibe Work: retain a model this person picked before Vibe's catalog replaced its built-in list.
[[models]]
name = "mistral-vibe-cli-latest"
provider = "mistral"
alias = "mistral-medium-3.5"
display_name = "Mistral Medium 3.5"
temperature = 1.0
input_price = 1.5
output_price = 7.5
cached_input_price = 0.15
thinking = "high"
thinking_levels = ["off", "high"]
supports_images = true
max_context_length = 262144
"#;

pub(super) fn is_builtin_medium(choice: &Choice) -> bool {
    choice.alias == ALIAS
        && choice.name == NAME
        && choice.provider == "mistral"
        && !choice.provider_key.is_empty()
}

fn failed() -> CoreError {
    CoreError::new(
        ErrorCode::Internal,
        "The saved Medium 3.5 model could not be kept in Vibe's configuration.",
    )
}

fn marker_exists(path: &Path) -> Result<bool, CoreError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata
            .file_type()
            .is_file()
            .then_some(true)
            .ok_or_else(failed),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err(failed()),
    }
}

fn ensure(home: &Path, sessions: &Path, explicit_pick: bool) -> Result<(), CoreError> {
    let path = home.join("config.toml");
    if !path.exists() {
        // A server awaiting first installation has no config yet. The next start retries.
        return Ok(());
    }
    let marker = sessions.join(MARKER);
    if !explicit_pick && marker_exists(&marker)? {
        return Ok(());
    }
    for _ in 0..2 {
        let before = crate::default_model::read(&path)?;
        let parsed: toml::Table = before.parse().map_err(|_| failed())?;
        let present = match parsed.get("models") {
            None => false,
            Some(toml::Value::Array(rows)) => rows
                .iter()
                .any(|row| row.get("alias").and_then(toml::Value::as_str) == Some(ALIAS)),
            Some(_) => return Err(failed()),
        };
        if !present {
            let after = format!("{before}{MODEL}");
            after.parse::<toml::Table>().map_err(|_| failed())?;
            if crate::default_model::read(&path)? != before {
                continue;
            }
            vw_platform::replace_private(&path, after.as_bytes()).map_err(|_| failed())?;
        }
        vw_platform::ensure_private_dir(sessions).map_err(|_| failed())?;
        vw_platform::replace_private(&marker, b"retained\n").map_err(|_| failed())?;
        tracing::debug!(
            alias = ALIAS,
            "saved built-in model is available in Vibe config"
        );
        return Ok(());
    }
    Err(CoreError::new(
        ErrorCode::Stale,
        "Vibe's model config changed while keeping this chat's choice. Try again.",
    ))
}

/// Migrate previously saved built-in picks before Vibe reads the catalog on launch.
pub(crate) fn prepare_builtin_models(home: &Path, sessions: Option<&Path>) {
    let Some(sessions) = sessions else { return };
    if Records::load(Some(sessions)).has_saved_builtin_medium()
        && let Err(error) = ensure(home, sessions, false)
    {
        tracing::warn!(%error, "saved built-in model could not be prepared before Vibe launch");
    }
}

/// A new pick keeps Vibe's built-in row in the user layer for later starts.
pub(super) fn picked(shared: &Shared, choice: &Choice) -> Result<(), CoreError> {
    if is_builtin_medium(choice)
        && let Some(sessions) = shared.config.sessions_dir.as_deref()
    {
        ensure(&shared.instructions.own_home, sessions, true)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removed_model_stays_removed_until_an_explicit_new_pick() {
        let root = tempfile::tempdir().unwrap();
        let sessions = root.path().join("sessions");
        std::fs::create_dir(&sessions).unwrap();
        let config = root.path().join("config.toml");
        let initial = "active_model = \"mistral-large-4-0\"\n";
        std::fs::write(&config, initial).unwrap();
        ensure(root.path(), &sessions, false).unwrap();
        assert!(std::fs::read_to_string(&config).unwrap().contains(ALIAS));
        std::fs::write(&config, initial).unwrap();
        ensure(root.path(), &sessions, false).unwrap();
        assert_eq!(std::fs::read_to_string(&config).unwrap(), initial);
        ensure(root.path(), &sessions, true).unwrap();
        assert!(std::fs::read_to_string(&config).unwrap().contains(ALIAS));
    }

    #[test]
    fn an_unbound_or_different_model_cannot_trigger_builtin_retention() {
        let mut choice = Choice {
            alias: ALIAS.into(),
            name: NAME.into(),
            provider: "mistral".into(),
            provider_key: String::new(),
            effort: "high".into(),
            display_name: "Mistral Medium 3.5".into(),
        };
        assert!(!is_builtin_medium(&choice));
        choice.provider_key = "bound".into();
        assert!(is_builtin_medium(&choice));
        choice.name = "a different model".into();
        assert!(!is_builtin_medium(&choice));
    }
}
