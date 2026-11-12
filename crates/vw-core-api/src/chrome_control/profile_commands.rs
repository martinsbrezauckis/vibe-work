//! Person-owned profile settings. Uses the private helper pipe to fence one identity and revoke its pairing;
//! commands return the resulting view only after helper acknowledgement. No browser directory is guessed.
use super::{ChromeControlRequest, agent_browser::Browser, commands::invalid, profiles};
use crate::{CommandResult, Core, CoreError, core::Shared};
use serde_json::json;
use std::sync::{PoisonError, atomic::Ordering};

impl Core {
    pub(super) async fn profile_setting(
        &self,
        origin: crate::Origin,
        request: ChromeControlRequest,
    ) -> Result<CommandResult, CoreError> {
        if crate::command_origin::debug_api_acts(&self.shared, origin) {
            return Err(invalid(
                "Browser profile settings require the user. Ask the user to make this change in Browser settings.",
            ));
        }
        let chrome = &self.shared.chrome_control;
        let _held = chrome.lifecycle.lock().await;
        let id = match &request {
            ChromeControlRequest::RenameProfile { profile_id, .. }
            | ChromeControlRequest::SetDefaultProfile { profile_id }
            | ChromeControlRequest::SetProfileEnabled { profile_id, .. }
            | ChromeControlRequest::ConnectProfile { profile_id }
            | ChromeControlRequest::RemoveProfile { profile_id } => profiles::canonical(profile_id),
            _ => {
                return Err(invalid(
                    "Unknown profile setting. Read Browser settings and try again.",
                ));
            }
        };
        let fence = save_setting(&self.shared, &id, &request)?;
        if fence {
            fence_profile(&self.shared, &id);
        }
        let needs_delivery = !matches!(
            request,
            ChromeControlRequest::RenameProfile { .. }
                | ChromeControlRequest::SetDefaultProfile { .. }
        );
        if matches!(request, ChromeControlRequest::ConnectProfile { .. })
            && !chrome.owner.is_running()
        {
            super::start_owner(&self.shared).await;
        }
        if needs_delivery {
            chrome.switch_generation.fetch_add(1, Ordering::SeqCst);
            let applied = chrome
                .owner
                .rpc(
                    "owner.browserAvailability",
                    super::switches::payload(&self.shared),
                )
                .await;
            if chrome.owner.is_running() && !applied.is_some_and(|result| result["applied"] == true)
            {
                super::owner::publish(&self.shared);
                return Err(invalid(
                    "Profile setting saved; release is unconfirmed. Reconnect this profile before using it.",
                ));
            }
        }
        if matches!(request, ChromeControlRequest::RemoveProfile { .. }) {
            let removed = chrome
                .owner
                .rpc("owner.profileUnpair", json!({"profileId":id}))
                .await;
            if !removed.is_some_and(|result| result["unpaired"] == true) {
                return Err(invalid(
                    "The profile is off, but its pairing could not be removed. Reconnect the helper and try Remove again.",
                ));
            }
        }
        if matches!(request, ChromeControlRequest::RemoveProfile { .. }) {
            finish_removal(&self.shared, &id).await?;
        }
        if matches!(request, ChromeControlRequest::ConnectProfile { .. }) {
            let answer = chrome.owner.call("chrome_status", json!({})).await;
            super::owner::update_pairing(&std::sync::Arc::downgrade(&self.shared), answer.value());
            if !profiles::view(&self.shared)
                .iter()
                .any(|row| row.profile_id == id && row.eligible)
            {
                return Err(invalid(&format!(
                    "{} is offline. Open this browser profile and enable its Vibe Work extension, then Connect.",
                    profiles::label(chrome, &id)
                )));
            }
        }
        super::owner::publish(&self.shared);
        Ok(CommandResult::ChromeControl {
            view: super::commands::view(&self.shared),
            offer: None,
        })
    }
}

pub(super) fn fence_profile(shared: &Shared, profile: &str) {
    let chrome = &shared.chrome_control;
    chrome.owner.approvals.cancel_profile(profile);
    crate::ask::tool_calls::cancel_browser_profile(shared, profile);
    let active = {
        let mut tasks = chrome.tasks.lock().unwrap_or_else(PoisonError::into_inner);
        let mut work = chrome.work.lock().unwrap_or_else(PoisonError::into_inner);
        let active: Vec<_> = tasks
            .iter()
            .filter(|(id, _)| {
                work.profile_of(id)
                    .map(str::to_owned)
                    .or_else(|| super::profile_bindings::session_profile(chrome, id))
                    .as_deref()
                    .is_some_and(|old| super::profile_ids::same(old, profile))
            })
            .map(|(id, chat)| (id.clone(), chat.clone()))
            .collect();
        for (id, chat) in &active {
            tasks.remove(id);
            work.ask_stop(id, chat);
        }
        active
    };
    for (id, _) in active {
        chrome
            .owner
            .control(json!({"action":"stop","sessionId":id}));
    }
}

fn save_setting(
    shared: &Shared,
    id: &str,
    request: &ChromeControlRequest,
) -> Result<bool, CoreError> {
    let chrome = &shared.chrome_control;
    let browser = Browser::from_profile(id);
    let mut next = chrome
        .profiles
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let record = next
        .records
        .get_mut(id)
        .filter(|record| !record.removed)
        .ok_or_else(|| invalid("This profile is no longer paired. Connect the profile again."))?;
    let fence = matches!(
        &request,
        ChromeControlRequest::SetProfileEnabled { enabled: false, .. }
            | ChromeControlRequest::RemoveProfile { .. }
    );
    match &request {
        ChromeControlRequest::RenameProfile { name, .. } => {
            let name = name.trim();
            if name.is_empty() || name.chars().count() > 80 || name.chars().any(char::is_control) {
                return Err(invalid("Use a profile name of 1–80 characters."));
            }
            record.name = Some(name.into());
        }
        ChromeControlRequest::SetDefaultProfile { .. } => {
            if !profiles::view(shared)
                .iter()
                .any(|row| row.profile_id == id && row.eligible)
            {
                return Err(invalid(
                    "This profile is unavailable. Connect it before choosing Default.",
                ));
            }
            next.defaults.insert(browser.id().into(), id.to_owned());
        }
        ChromeControlRequest::SetProfileEnabled { enabled, .. } => record.enabled = *enabled,
        ChromeControlRequest::ConnectProfile { .. } => record.enabled = true,
        ChromeControlRequest::RemoveProfile { .. } => {
            record.enabled = false;
        }
        _ => unreachable!("validated profile command"),
    }
    if fence {
        next.defaults.retain(|_, selected| selected != id);
        *next.generations.entry(id.to_owned()).or_default() += 1;
    }
    next.save(chrome.dir()).map_err(|reason| invalid(&reason))?;
    *chrome
        .profiles
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = next;
    Ok(fence)
}

/// Once the trust pin is revoked, permit a fresh native ceremony without re-enabling action authority.
async fn finish_removal(shared: &Shared, id: &str) -> Result<(), CoreError> {
    let chrome = &shared.chrome_control;
    {
        let mut saved = chrome
            .profiles
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut next = saved.clone();
        if let Some(record) = next.records.get_mut(id) {
            record.removed = true;
        }
        next.save(chrome.dir()).map_err(|reason| invalid(&reason))?;
        *saved = next;
    }
    chrome.switch_generation.fetch_add(1, Ordering::SeqCst);
    let applied = chrome
        .owner
        .rpc(
            "owner.browserAvailability",
            super::switches::payload(shared),
        )
        .await;
    if chrome.owner.is_running() && !applied.is_some_and(|result| result["applied"] == true) {
        return Err(invalid(
            "Pairing removed; reconnect is unconfirmed. Restart Vibe Work before pairing this profile again.",
        ));
    }
    Ok(())
}
