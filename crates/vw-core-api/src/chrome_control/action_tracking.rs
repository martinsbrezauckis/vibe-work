//! Adapts completed helper receipts to the shared host action timeline, before model wrapping and pagination.
//! Called only by the input output boundary. Identity comes from the admitted lease; no page text is retained.
use crate::core::Shared;
use serde_json::Value;
use std::sync::PoisonError;
use vw_preview::action_tracking::{
    ActionGeometry, ActionKind, ActionPoint, ActionSource, ActionTracker, SourceBrowser,
};

impl Shared {
    /// Native and external producers always use this same timeline. Headless cores allocate it lazily.
    pub(crate) fn action_tracker(&self) -> &ActionTracker {
        self.preview.as_ref().map_or_else(
            || {
                self.chrome_control
                    .actions
                    .get_or_init(ActionTracker::default)
            },
            |hub| hub.service.action_tracker().as_ref(),
        )
    }
}

pub(super) fn record(shared: &Shared, chat: &str, request: &Value, result: &Value) {
    let receipt = &result["structuredContent"];
    if result["isError"] == true
        || receipt["status"] != "completed"
        || !receipt["error"].is_null()
        || receipt["action"] != request["action"]
        || super::output_cache::is_continuation(request)
    {
        return;
    }
    let kind = match request["action"].as_str() {
        Some(
            "page.click" | "page.double_click" | "page.right_click" | "page.triple_click"
            | "page.middle_click",
        ) => ActionKind::Click,
        Some("page.type") => ActionKind::Type,
        Some("page.check" | "page.uncheck") => ActionKind::Check,
        Some("page.select") => ActionKind::Select,
        Some("page.key") => ActionKind::Key,
        Some("page.scroll") => ActionKind::Scroll,
        _ => return,
    };
    let Some(lease) = request["sessionId"].as_str() else {
        return;
    };
    if receipt["sessionId"].as_str() != Some(lease) {
        return;
    }
    let tasks = shared
        .chrome_control
        .tasks
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if tasks.get(lease).map(String::as_str) != Some(chat) {
        return;
    }
    let work = shared
        .chrome_control
        .work
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if work.chat_of(lease) != Some(chat) || work.state_of(lease, chat) != Some("active") {
        return;
    }
    let Some(profile) = work.profile_of(lease) else {
        return;
    };
    let browser = if profile.starts_with("edge:") {
        SourceBrowser::Edge
    } else if profile.starts_with("safari:") {
        SourceBrowser::Safari
    } else if profile.starts_with("firefox:") {
        SourceBrowser::Firefox
    } else {
        SourceBrowser::Chrome
    };
    let data = &receipt["data"];
    let source = ActionSource {
        browser,
        profile_id: Some(profile.into()),
        browser_session_id: Some(lease.into()),
        view_id: data["tabId"]
            .as_u64()
            .or_else(|| request["args"]["tabId"].as_u64())
            .map(|id| id.to_string()),
        document_id: data["documentId"]
            .as_str()
            .filter(|id| id.len() <= 256)
            .map(str::to_owned),
    };
    // Existing coordinate requests are known after a completed receipt, but a receipt without a measured
    // top-level viewport cannot place them on an app surface. Ref-based receipts do not guarantee geometry.
    let point = if kind == ActionKind::Click
        && request["args"]["frameId"].as_u64().is_none_or(|id| id == 0)
    {
        request["args"]["x"]
            .as_f64()
            .zip(request["args"]["y"].as_f64())
            .map(|(x, y)| ActionPoint { x, y })
    } else {
        None
    };
    shared.action_tracker().record(
        chat.into(),
        kind,
        source,
        ActionGeometry {
            point,
            ..Default::default()
        },
    );
}

#[cfg(test)]
#[path = "action_tracking_tests.rs"]
mod tests;
