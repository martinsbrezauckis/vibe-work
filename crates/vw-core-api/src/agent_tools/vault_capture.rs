//! Direct browser-to-vault saves prepared in the authenticated pre-tool hook.
//! Values stay in the browser buffer and key service; cards and receipts contain only names and source metadata.
use crate::{
    ask::tool_calls::{receipts::Execution, vault_save},
    core::Shared,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use vw_keys::{CaptureChoice, EntryPermission};
use vw_vault::SecretString;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    #[serde(rename = "ref")]
    reference: String,
    name: String,
    #[serde(default)]
    tab: Option<String>,
}
pub(super) fn vault_error(shared: &Shared, error: &vw_keys::KeyError) -> String {
    if vw_keys::is_locked(error) {
        shared.emit(crate::CoreEvent::KeyNeeded {
            reason: "unlock".into(),
        });
        vw_vocabulary::texts::AGENT_VAULT_LOCKED.into()
    } else {
        "The vault save could not complete or its entry changed; ask the user to check the vault and try again.".into()
    }
}
pub(super) fn permission(shared: &Shared, owner: &str) -> Result<EntryPermission, String> {
    match shared.access.remembered(owner).as_deref() {
        Some("auto-approve") => Ok(EntryPermission::Allowed),
        Some("ask" | "accept-edits") => Ok(EntryPermission::AskEachTime),
        _ => Err(format!(
            "This chat mode cannot save secrets; use {}, {} or {}.",
            vw_vocabulary::label("accessModes", "ask"),
            vw_vocabulary::label("accessModes", "accept-edits"),
            vw_vocabulary::label("accessModes", "auto-approve"),
        )),
    }
}
pub(super) async fn replace_allowed(
    shared: &Shared,
    owner: &str,
    keys: &vw_keys::KeyService,
    plan: &vw_keys::CapturePlan,
) -> Result<bool, String> {
    check_scope(shared, owner, plan)?;
    let Some(existing) = plan.existing().filter(|s| s.own) else {
        return Ok(false);
    };
    let project = shared.entry_projects.get(owner)?;
    let policy = project
        .policy(keys, &existing.id)
        .await
        .map_err(|e| vault_error(shared, &e))?;
    Ok(policy.permission != EntryPermission::Never)
}

/// A capture can replace an existing item; it must not change the Debug bearer in a lower mode either.
pub(super) fn check_scope(
    shared: &Shared,
    owner: &str,
    plan: &vw_keys::CapturePlan,
) -> Result<(), String> {
    if let Some(existing) = plan.existing() {
        crate::agent_access::check_item(shared, owner, &existing.id)?;
    }
    Ok(())
}
pub(super) async fn prepare(
    shared: &Arc<Shared>,
    owner: &str,
    args: &Value,
) -> Result<Execution, String> {
    crate::agent_access::check_for(shared, vault_save::KIND, owner)?;
    let input: Input = serde_json::from_value(args.clone()).map_err(
        |_| "A vault capture needs ref and name, with optional tab; point at a browser_read ref.",
    )?;
    if !vw_vault::project_secret::valid_name(&input.name) || input.reference.len() > 100 {
        return Err(
            "The ref or name is invalid; use a browser_read ref and a short entry name.".into(),
        );
    }
    let default = permission(shared, owner)?;
    let keys = shared
        .config
        .keys
        .as_ref()
        .ok_or(vw_vocabulary::texts::AGENT_VAULT_MISSING)?;
    // A locked vault is checked before reading a value or presenting a save card.
    let plan = keys
        .plan_capture(input.name.clone())
        .await
        .map_err(|e| vault_error(shared, &e))?;
    let duplicate = plan.duplicate();
    let replace = replace_allowed(shared, owner, keys, &plan).await?;
    let hub = crate::previews::hub(shared)
        .map_err(|_| "The browser is unavailable; open a page first.")?;
    let page = crate::browser_tabs::agent_page(&hub.service, owner, input.tab.as_deref())
        .map_err(|e| e.message)?;
    super::browser_tools::check_current(shared, owner, &page)?;
    crate::browser_tabs::front_page(&hub.service, owner, &page)
        .await
        .map_err(|_| "The page could not open; try again.")?;
    let mut cancelled = hub.browser.sessions().cancel_signal(&page);
    let captured = hub
        .browser
        .capture_agent_secret(&page, &input.reference)
        .await?;
    // Prevent accidentally naming the entry after its value, including a value already typed in chat.
    if input.name.contains(captured.value.as_str()) {
        return Err("The entry name contains the captured value; choose a different name.".into());
    }
    let choice = if duplicate || default != EntryPermission::Allowed {
        tokio::select! {
            biased;
            _ = cancelled.changed() => return Err("The browser action ended; read the page again before saving.".into()),
            answer = vault_save::ask(
                shared, owner, &input.name, &captured.origin, duplicate,
                replace,
                vw_agent_tools::wire::tools::BROWSER_CAPTURE_SECRET,
            ) => answer?,
        }
    } else {
        CaptureChoice::New
    };
    let shared = Arc::downgrade(shared);
    let owner = owner.to_owned();
    Ok(Box::pin(async move {
        let shared = shared.upgrade().ok_or("Vibe Work closed; try again.")?;
        crate::agent_access::check(&shared)?;
        if permission(&shared, &owner)? != default {
            return Err(
                "The chat access mode changed; repeat the save under its current mode.".into(),
            );
        }
        super::browser_tools::check_current(&shared, &owner, &page)?;
        let keys = shared
            .config
            .keys
            .as_ref()
            .ok_or("The vault is unavailable; try again.")?;
        if choice == CaptureChoice::Replace
            && !replace_allowed(&shared, &owner, keys, &plan).await?
        {
            return Err("The entry's access changed; request the save again.".into());
        }
        let stored = keys
            .store_capture(
                plan,
                choice,
                SecretString::exact(captured.value.as_str()),
                Some(captured.origin.clone()),
                default,
            )
            .await
            .map_err(|e| vault_error(&shared, &e))?;
        shared.emit(crate::CoreEvent::SecretsChanged {});
        Ok(json!({"name":stored.secret.name,"source":"browser",
            "permission":vw_vocabulary::label("vaultPermissions", stored.permission.as_str())}))
    }))
}
