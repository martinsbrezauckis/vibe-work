//! Person-menu context decoding and picker claims. The signed owner observer queues metadata; the composer claims it once.
use super::{ChromeSelection, ChromeShareKind, content, selection::PendingSelection};
use crate::{CommandResult, CoreError, ErrorCode, core::Shared};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Share {
    kind: String,
    page_url: String,
    page_title: String,
    client_session_id: Option<String>,
    url: Option<String>,
    handover: Option<Handover>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Handover {
    profile_id: String,
    browser_session_id: String,
    tab: SharedTab,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SharedTab {
    tab_id: u32,
    window_id: u32,
    document_id: String,
    url: String,
    title: String,
    origin: String,
    #[serde(rename = "active")]
    _active: bool,
    #[serde(rename = "groupId")]
    _group_id: Option<u32>,
}

fn bounded(value: &str, max: usize) -> bool {
    !value.is_empty() && value.encode_utf16().count() <= max
}

pub(super) fn decode(params: &Value) -> Option<PendingSelection> {
    let id = params["shareId"].as_str().filter(|id| bounded(id, 256))?;
    let share: Share = serde_json::from_value(params["share"].clone()).ok()?;
    if !bounded(&share.page_url, 16_384)
        || share.page_title.encode_utf16().count() > 4096
        || share
            .client_session_id
            .as_ref()
            .is_some_and(|id| !bounded(id, 256))
    {
        return None;
    }
    let page = url::Url::parse(&share.page_url).ok()?;
    if !matches!(page.scheme(), "http" | "https" | "file" | "data")
        && page.as_str() != "about:blank"
    {
        return None;
    }
    let (kind, text) = match share.kind.as_str() {
        "tab" => {
            let handover = share.handover.as_ref()?;
            let origin = match page.scheme() {
                "data" => "data:".to_owned(),
                "file" => "file://".to_owned(),
                "about" => "about:blank".to_owned(),
                _ => page.origin().ascii_serialization(),
            };
            if share.url.is_some()
                || !bounded(&handover.profile_id, 128)
                || !bounded(&handover.browser_session_id, 128)
                || !bounded(&handover.tab.document_id, 256)
                || handover.tab.url != share.page_url
                || handover.tab.title != share.page_title
                || handover.tab.origin != origin
                || handover.tab.tab_id > i32::MAX as u32
                || handover.tab.window_id > i32::MAX as u32
            {
                return None;
            }
            (ChromeShareKind::Page, String::new())
        }
        "link" | "image" => {
            if share.handover.is_some() {
                return None;
            }
            let resource = share.url.as_ref().filter(|url| bounded(url, 16_384))?;
            url::Url::parse(resource).ok()?;
            (
                if share.kind == "link" {
                    ChromeShareKind::Link
                } else {
                    ChromeShareKind::Image
                },
                resource.clone(),
            )
        }
        _ => return None,
    };
    let mut marked = json!({"structuredContent":{"kind":kind,"pageUrl":share.page_url,"pageTitle":share.page_title,"url":share.url}});
    content::mark(&mut marked);
    Some(PendingSelection {
        selection: ChromeSelection {
            selection_id: id.into(),
            session_id: share.client_session_id,
            title: share.page_title,
            url: share.page_url,
            text,
            model_text: marked["structuredContent"].to_string(),
            kind: Some(kind),
        },
        handover: share.handover,
        builtin: None,
    })
}

pub(super) async fn claim(
    shared: &Shared,
    pending: PendingSelection,
) -> Result<ChromeSelection, CoreError> {
    if let Some(proof) = pending.builtin {
        return super::builtin_share::claim(shared, proof, pending.selection).await;
    }
    let mut selection = pending.selection;
    selection.keep_live_chat(|id| shared.store.view(id).is_some());
    let Some(handover) = pending.handover else {
        return Ok(selection);
    };
    let recipient = selection
        .session_id
        .clone()
        .unwrap_or_else(|| shared.new_chat.owner_id());
    let expected = json!({"browserSessionId":handover.browser_session_id,"windowId":handover.tab.window_id,"documentId":handover.tab.document_id,"url":handover.tab.url});
    let result = super::tabs::pick_expected(
        shared,
        &shared.chrome_control,
        &handover.profile_id,
        handover.tab.tab_id,
        Some(&recipient),
        Some(expected),
    )
    .await?;
    let CommandResult::ChromeSelection {
        selection: Some(mut picked),
    } = result
    else {
        return Err(CoreError::new(
            ErrorCode::InvalidCommand,
            "The shared page could not be read. Choose Ask Vibe Work on that page again.",
        ));
    };
    picked.selection_id = selection.selection_id;
    picked.session_id = selection.session_id;
    picked.kind = Some(ChromeShareKind::Page);
    Ok(picked)
}
