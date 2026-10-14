//! Saves a user's browser selection in the project vault.
//!
//! Depends on the browser's one-use choice, the vault and native confirmer. Called by browser commands.
//! The chosen value stays in native memory, never in command answers, events, or log fields.

use serde_json::{Value, json};
use vw_browser::Browser;
use vw_gate::{ConfirmationRequest, Confirmer, Requester};
use vw_vault::SecretString;
use vw_vault::project_secret;

use crate::core::Shared;
use crate::event::CoreEvent;

/// The native confirmation method.
pub(crate) const METHOD: &str = "vibe-work/captureSecret";

fn consent_request(
    title: &str,
    page: &str,
    element: &str,
    name: &str,
    length: usize,
    replaces: bool,
) -> ConfirmationRequest {
    ConfirmationRequest {
        method: METHOD.to_owned(),
        requester: Requester::Window,
        summary: format!(
            "Save the {length}-character value you selected in the {} on {} for session {} to your vault as {}{}? The agent never sees the value; it can later ask to use it in a command.",
            vw_gate::dialog_quote(element),
            vw_gate::dialog_quote(page),
            vw_gate::dialog_quote(title),
            vw_gate::dialog_quote(name),
            if replaces {
                " (replacing the value stored under that name)"
            } else {
                ""
            },
        ),
        params: json!({
            "session": title,
            "page": page,
            "element": element,
            "storeAs": name,
            "valueLength": length,
            "replaces": replaces,
        }),
        always: None,
    }
}

/// Starts a user choice and returns only a one-use handle and safe metadata.
///
/// # Errors
/// A denial when a session or page is absent, the selection is empty, or the picker is cancelled.
pub(crate) async fn choose(
    shared: &Shared,
    driver: &Browser,
    session_id: &str,
    mode: &str,
) -> Result<crate::browser_view::BrowserResult, crate::error::CoreError> {
    let owner = crate::previews::hub(shared)?
        .service
        .owner_session_id(session_id)
        .ok_or_else(|| denied("browser page has no session owner".to_owned()))?;
    if shared.store.view(&owner).is_none() {
        return Err(crate::error::CoreError::not_loaded(&owner));
    }
    let choice = driver
        .choose_secret(session_id, mode)
        .await
        .map_err(denied)?;
    Ok(crate::browser_view::BrowserResult::SecretChoice {
        choice: crate::browser_view::SecretChoiceView {
            handle: choice.handle,
            suggested_name: choice.suggested_name,
            page: choice.page,
            element: choice.element,
            value_length: choice.value_length,
        },
    })
}

fn denied(message: String) -> crate::error::CoreError {
    crate::error::CoreError::new(crate::error::ErrorCode::Denied, message)
}

/// Saves a one-use user choice after the native confirmation.
///
/// # Errors
/// A denial for a stale choice, invalid name, locked vault, or declined confirmation.
pub(crate) async fn save(
    shared: &Shared,
    driver: &Browser,
    session_id: &str,
    handle: &str,
    name: &str,
) -> Result<crate::browser_view::BrowserResult, crate::error::CoreError> {
    let owner = crate::previews::hub(shared)?
        .service
        .owner_session_id(session_id)
        .ok_or_else(|| denied("browser page has no session owner".to_owned()))?;
    if shared.store.view(&owner).is_none() {
        return Err(crate::error::CoreError::not_loaded(&owner));
    }
    let name = name.trim();
    if !project_secret::valid_name(name) {
        return Err(denied("name must be 1 to 80 characters of text".to_owned()));
    }
    let keys = shared
        .config
        .keys
        .as_ref()
        .ok_or_else(|| denied("Vibe Work has no vault connected".to_owned()))?;
    let existing = keys.project_secrets().await.map_err(|e| {
        if vw_keys::is_locked(&e) {
            denied("the vault is locked: unlock it in Settings, then try again".to_owned())
        } else {
            denied(format!("the vault could not be read ({e})"))
        }
    })?;
    let same = existing
        .iter()
        .find(|secret| secret.name.eq_ignore_ascii_case(name));
    if same.is_some_and(|secret| !secret.own) {
        return Err(denied(format!(
            "a secret of another app is already named '{name}': choose another name"
        )));
    }
    let replaces = same.is_some();
    let captured = driver
        .take_secret_choice(session_id, handle)
        .await
        .map_err(denied)?;
    let value = SecretString::new(captured.value.as_str());
    let length = value.expose().chars().count();
    let title = shared
        .names
        .title_of(&shared.store, &owner)
        .unwrap_or(owner);
    let request = consent_request(
        &title,
        &captured.origin,
        &captured.element,
        name,
        length,
        replaces,
    );
    if !shared.confirmer.confirm(&request).await.approved() {
        return Err(denied(
            "Not stored: the user declined the vault save".to_owned(),
        ));
    }
    let stored = keys
        .store_secret_labelled(name.to_owned(), value, Some(captured.origin), None)
        .await
        .map_err(|e| denied(format!("Not stored: {e}")))?;
    shared.emit(CoreEvent::SecretsChanged {});
    tracing::info!(
        replaced = stored.replaced,
        "a user-selected key was stored in the vault"
    );
    let answer: Value = json!({
        "stored": stored.secret.name,
        "id": stored.secret.id,
        "length": length,
        "replaced": stored.replaced,
    });
    Ok(crate::browser_view::BrowserResult::SecretStored { stored: answer })
}

#[cfg(test)]
mod tests {
    use super::consent_request;

    #[test]
    fn capture_summary_carries_safe_metadata() {
        let request = consent_request(
            "Session",
            "https://example.test",
            "password field",
            "Login",
            21,
            true,
        );
        for fact in [
            "21-character",
            "Session",
            "https://example.test",
            "password field",
            "Login",
            "replacing",
        ] {
            assert!(request.summary.contains(fact), "missing {fact}");
        }
        assert!(!request.summary.contains("vibe-work/captureSecret"));
        let title = "\"A\\B\"\nC";
        let escaped = consent_request(title, "page", "element", "name", 21, false);
        assert_eq!(
            escaped.params["session"], title,
            "metadata is quoted only by its renderer"
        );
        assert!(escaped.summary.contains(&vw_gate::dialog_quote(title)));
    }
}
