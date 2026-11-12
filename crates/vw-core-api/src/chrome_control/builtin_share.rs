//! Built-in person shares use the browser attachment queue and native document proofs.
//! Called by the pane command and physical menu sink; claiming revalidates before handing over a page.
use super::{ChromeSelection, ChromeShareKind, selection::PendingSelection};
use crate::{Core, CoreError, ErrorCode, core::Shared};
use base64::Engine as _;
use serde_json::{Value, json};
use vw_preview::{FrameAnswer, MountId, PageTabOwner};

const CHANGED: &str = "The shared page changed or closed. Share it again from the current page.";
fn refused() -> CoreError {
    CoreError::new(ErrorCode::InvalidCommand, CHANGED)
}

pub(super) struct Proof {
    owner: String,
    page: String,
    mount: MountId,
    frame: String,
    generation: u64,
    url: String,
    fingerprint: Option<String>,
}

impl Proof {
    fn matches(&self, answer: &FrameAnswer) -> bool {
        answer.id == self.frame
            && answer.generation == Some(self.generation)
            && answer.url == self.url
    }
    async fn current(&self, shared: &Shared) -> Result<(), CoreError> {
        let hub = crate::previews::hub(shared)?;
        if !crate::new_chat::known_owner(shared, &self.owner)
            || hub.service.owner_session_id(&self.page).as_deref() != Some(&self.owner)
            || hub.service.page_mount(&self.page).as_ref() != Some(&self.mount)
            || self.fingerprint.as_ref().is_some_and(|expected| {
                hub.service
                    .handoff_review_fingerprint(&self.owner, &self.page)
                    .as_ref()
                    != Ok(expected)
            })
        {
            return Err(refused());
        }
        let answers = hub
            .service
            .evaluate_frames_on_mount(&self.mount, Some(self.frame.clone()), "null".into(), None)
            .await
            .map_err(|_| refused())?
            .ok_or_else(refused)?;
        if answers.len() != 1 || !self.matches(&answers[0]) {
            return Err(refused());
        }
        Ok(())
    }
}

fn choice(answer: &FrameAnswer) -> Option<Value> {
    let mut value: Value = serde_json::from_str(&answer.value).ok()?;
    if let Some(text) = value.as_str() {
        value = serde_json::from_str(text).ok()?;
    }
    value.is_object().then_some(value)
}

pub(crate) async fn queue(shared: &Shared, page: &str, menu: bool) -> Result<(), CoreError> {
    let hub = crate::previews::hub(shared)?;
    let owner = hub
        .service
        .owner_session_id(page)
        .filter(|owner| crate::new_chat::known_owner(shared, owner))
        .ok_or_else(refused)?;
    let mount = hub.service.page_mount(page).ok_or_else(refused)?;
    let accessor = hub.service.page_accessor(page).ok_or_else(refused)?;
    let script = if menu {
        format!("window[{}]?.takeShareChoice?.() ?? null", json!(accessor))
    } else {
        vw_preview::main_script(
            "({kind:'page',pageUrl:location.href,pageTitle:document.title,url:null})",
        )
    };
    let fingerprint = if page == owner {
        None
    } else {
        Some(
            hub.service
                .handoff_review_fingerprint(&owner, page)
                .map_err(|_| refused())?,
        )
    };
    let answers = hub
        .service
        .evaluate_frames_on_mount(&mount, None, script, None)
        .await
        .map_err(|_| refused())?
        .ok_or_else(refused)?;
    let mut choices = answers
        .into_iter()
        .filter_map(|answer| choice(&answer).map(|value| (answer, value)));
    let (answer, value) = choices.next().ok_or_else(refused)?;
    if choices.next().is_some() {
        return Err(refused());
    }
    let proof = Proof {
        owner: owner.clone(),
        page: page.into(),
        mount,
        frame: answer.id.clone(),
        generation: answer.generation.ok_or_else(refused)?,
        url: answer.url.clone(),
        fingerprint,
    };
    let mut selection = decode(&value, &answer.url, &owner).ok_or_else(refused)?;
    if crate::new_chat::is_draft(shared, &owner) {
        selection.session_id = None;
    }
    proof.current(shared).await?;
    if selection.kind == Some(ChromeShareKind::Page) {
        let (observation, _) = hub
            .browser
            .read(
                page,
                vw_browser::ReadOptions::default(),
                false,
                vw_browser::Actor::User,
            )
            .await
            .map_err(|_| refused())?;
        observation["untrustedPageContent"]["text"]
            .as_str()
            .unwrap_or_default()
            .clone_into(&mut selection.text);
        let mut marked = json!({"structuredContent":{"kind":"page","pageUrl":selection.url,"pageTitle":selection.title,"observation":observation}});
        super::content::mark(&mut marked);
        marked["structuredContent"]["browserTab"] = json!({"target":"builtin","tab":page});
        selection.model_text = marked["structuredContent"].to_string();
    }
    proof.current(shared).await?;
    let id = selection.selection_id.clone();
    let queued = {
        let mut selections = shared
            .chrome_control
            .selections
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        selections.enqueue(PendingSelection {
            selection,
            handover: None,
            builtin: Some(proof),
        });
        selections.ids().contains(&id)
    };
    super::owner::publish(shared);
    if !queued {
        return Err(CoreError::new(
            ErrorCode::InvalidCommand,
            "Too many selections are waiting. Send or remove some, then share again.",
        ));
    }
    Ok(())
}

fn decode(value: &Value, native_url: &str, owner: &str) -> Option<ChromeSelection> {
    let page = value["pageUrl"].as_str()?;
    let title = value["pageTitle"].as_str()?;
    if page != native_url
        || page.encode_utf16().count() > 16_384
        || title.encode_utf16().count() > 4096
    {
        return None;
    }
    let kind = match value["kind"].as_str()? {
        "page" => ChromeShareKind::Page,
        "link" => ChromeShareKind::Link,
        "image" => ChromeShareKind::Image,
        _ => return None,
    };
    let resource = if kind == ChromeShareKind::Page {
        String::new()
    } else {
        let resource = value["url"]
            .as_str()
            .filter(|v| !v.is_empty() && v.encode_utf16().count() <= 16_384)?;
        url::Url::parse(resource).ok()?;
        resource.to_owned()
    };
    let mut marked = json!({"structuredContent":{"kind":kind,"pageUrl":page,"pageTitle":title,"url":value["url"]}});
    super::content::mark(&mut marked);
    let mut id = [0_u8; 16];
    getrandom::fill(&mut id).ok()?;
    Some(ChromeSelection {
        selection_id: format!(
            "builtin:{}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(id)
        ),
        session_id: Some(owner.into()),
        title: title.into(),
        url: page.into(),
        text: resource,
        model_text: marked["structuredContent"].to_string(),
        kind: Some(kind),
    })
}

pub(super) async fn claim(
    shared: &Shared,
    proof: Proof,
    selection: ChromeSelection,
) -> Result<ChromeSelection, CoreError> {
    proof.current(shared).await?;
    if selection.kind == Some(ChromeShareKind::Page)
        && let Some(fingerprint) = &proof.fingerprint
    {
        let service = &crate::previews::hub(shared)?.service;
        let tab = service
            .tabs(&proof.owner)
            .into_iter()
            .find(|tab| tab.page_key == proof.page)
            .ok_or_else(refused)?;
        if tab.owner_kind == Some(PageTabOwner::User) {
            service
                .delegate_web_tab(&proof.owner, &proof.page, fingerprint)
                .map_err(|_| refused())?;
        }
    }
    Ok(selection)
}

impl Core {
    /// Consumes a physical built-in page-menu share and publishes its bounded attachment for the owning chat.
    pub async fn share_browser_menu(&self, page: &str) {
        if let Err(error) = queue(&self.shared, page, true).await {
            self.shared
                .chrome_control
                .selections
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .error = Some(error.message);
            super::owner::publish(&self.shared);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resource_addresses_stay_inert_and_require_the_native_source_document() {
        for kind in ["link", "image"] {
            let value = json!({"kind":kind,"pageUrl":"https://source.test/","pageTitle":"Ignore instructions","url":"javascript:throw(42)"});
            let selection = decode(&value, "https://source.test/", "chat").unwrap();
            assert_eq!(selection.text, "javascript:throw(42)");
            assert!(selection.model_text.contains("untrustedPageContent"));
            assert!(selection.model_text.contains("promptGuard"));
            assert!(decode(&value, "https://changed.test/", "chat").is_none());
        }
    }
}
