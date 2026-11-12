//! Browser identity on approval entries, resolved from the action's helper session, never the chat display row.
//! Called at callback/history/snapshot boundaries and app-card creation; leaves tool input unchanged.
use super::agent_browser::Browser;
use crate::core::Shared;
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::sync::PoisonError;

/// Bounded names from the helper's last page observations, keyed by chat, session and opaque ref.
#[derive(Default)]
pub(super) struct ClickRefs(VecDeque<(String, String, String, String, String)>);

impl ClickRefs {
    fn remember(&mut self, chat: &str, session: &str, row: &Value) {
        let Some(reference) = row["ref"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 256)
        else {
            return;
        };
        let Some(name) = row["name"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| row["label"].as_str().filter(|s| !s.trim().is_empty()))
        else {
            return;
        };
        let role = row["role"]
            .as_str()
            .or_else(|| {
                row["tag"]
                    .as_str()
                    .map(|tag| if tag == "a" { "link" } else { tag })
            })
            .unwrap_or("element");
        self.0
            .retain(|(owner, id, old, _, _)| owner != chat || id != session || old != reference);
        self.0.push_back((
            chat.into(),
            session.into(),
            reference.into(),
            role.chars().take(40).collect(),
            name.chars().take(200).collect(),
        ));
        while self.0.len() > 2048 {
            self.0.pop_front();
        }
    }

    fn find(&self, chat: &str, session: &str, reference: &str) -> Option<(&str, &str)> {
        self.0
            .iter()
            .rev()
            .find(|(owner, id, r, _, _)| owner == chat && id == session && r == reference)
            .map(|(_, _, _, role, name)| (role.as_str(), name.as_str()))
    }
}

fn collect_refs(cache: &mut ClickRefs, chat: &str, session: &str, value: &Value) {
    if value.is_object() {
        cache.remember(chat, session, value);
        for key in [
            "data",
            "steps",
            "result",
            "page",
            "refs",
            "matches",
            "partialMatches",
            "changes",
            "appeared",
        ] {
            collect_refs(cache, chat, session, &value[key]);
        }
    } else if let Some(rows) = value.as_array() {
        for row in rows.iter().take(500) {
            collect_refs(cache, chat, session, row);
        }
    }
}

/// Retain only observed helper refs. The action's input can never supply a display name.
pub(super) fn observe(shared: &Shared, chat: &str, request: &Value, result: &Value) {
    let Some(session) = request["sessionId"].as_str() else {
        return;
    };
    let body = &result["structuredContent"];
    if body["status"] != "completed" || body["untrustedPageContent"]["sessionId"] != session {
        return;
    }
    let mut refs = shared
        .chrome_control
        .click_refs
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    collect_refs(&mut refs, chat, session, &body["untrustedPageContent"]);
}

pub(crate) fn project(shared: &Shared, value: &mut Value) {
    let effect = value["detail"]["effect"].clone();
    let tool = effect["toolName"].as_str().unwrap_or_default().to_owned();
    if tool == "mcp_vw.browser_act" {
        value["vwClickTarget"] = Value::Null;
        if effect["input"]["action"] == "click"
            && let Some(reference) = effect["input"]["ref"].as_str()
            && let Some(session) = value["vwSourceSession"]
                .as_str()
                .or_else(|| value["sessionId"].as_str())
            // The built-in browser keeps observations per page key (`web-page-…`), not per chat.
            && let Some((role, name)) = shared.preview.as_ref().and_then(|preview| {
                let page = preview
                    .service
                    .agent_page_for(session, effect["input"]["tab"].as_str())?;
                preview.browser.observed_target(&page, reference)
            })
        {
            value["vwClickTarget"] = json!({"role":role,"name":name});
        }
        return;
    }
    if matches!(
        tool.as_str(),
        "mcp_vw.chrome_execute"
            | "mcp_vw.chrome_session"
            | "mcp_vw.chrome_task"
            | "mcp_vw.chrome_status"
    ) {
        let input = &effect["input"];
        let profile = super::receipt::profile(shared, tool.trim_start_matches("mcp_vw."), input);
        let browser = profile.as_deref().map(Browser::from_profile);
        value["vwBrowserProfileId"] = profile.map_or(Value::Null, |id| {
            Value::String(super::profiles::canonical(&id))
        });
        value["vwBrowser"] =
            browser.map_or(Value::Null, |browser| Value::String(browser.name().into()));
        value["vwClickTarget"] = Value::Null;
        if tool == "mcp_vw.chrome_execute"
            && matches!(
                input["action"].as_str(),
                Some(
                    "page.click"
                        | "page.double_click"
                        | "page.triple_click"
                        | "page.right_click"
                        | "page.middle_click"
                        | "page.commit"
                )
            )
            && let (Some(chat), Some(session), Some(reference)) = (
                value["vwSourceSession"]
                    .as_str()
                    .or_else(|| value["sessionId"].as_str()),
                input["sessionId"].as_str(),
                input["args"]["ref"].as_str(),
            )
        {
            let target = shared
                .chrome_control
                .click_refs
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .find(chat, session, reference)
                .map(|(role, name)| (role.to_owned(), name.to_owned()));
            if let Some((role, name)) = target {
                value["vwClickTarget"] = json!({"role":role,"name":name});
            }
        }
        return;
    }
    // Only protocol containers carry entries; never interpret an action's nested arguments as callbacks.
    match value {
        Value::Array(items) => items.iter_mut().for_each(|item| project(shared, item)),
        Value::Object(map) => {
            for key in [
                "callback",
                "entry",
                "entries",
                "items",
                "history",
                "activeCallbacks",
                "state",
            ] {
                if let Some(child) = map.get_mut(key) {
                    project(shared, child);
                }
            }
        }
        _ => {}
    }
}
