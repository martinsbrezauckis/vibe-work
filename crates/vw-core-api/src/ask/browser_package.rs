//! Browser session grants bind the chat, disclosed actions and tier. Descriptions reuse the card's verbs.
//! Called only for core-created Ask cards; native vault grants remain independent.
//! Depends on: browser classification and Ask grant matching.
//! Called by: core-created browser permission cards.
use super::grants::Scopeable;
use serde_json::{Value, json};
use vw_ask::{CoversPart, GrantPermission, SessionGrant, chrome::Tier};

const SCOPE: &str = "vibe_work_browser";

fn requested_actions(
    entry: &Value,
    input: &Value,
    builtin: bool,
    tier: Tier,
    actions: &[&str],
) -> Option<Vec<String>> {
    if !builtin && input.get("steps").is_some() {
        if input.get("action").is_some() || entry["detail"]["vwBrowserAction"].is_string() {
            return None;
        }
        let steps = crate::chrome_control::batch::steps(input).ok()?;
        let mut requested = Vec::new();
        for step in steps {
            let required = vw_ask::chrome::actions::ask_required("chrome_execute", &step)?;
            if required > tier {
                return None;
            }
            if required > Tier::Ask {
                let action = step["action"]
                    .as_str()
                    .filter(|action| actions.contains(action))?;
                if !requested.iter().any(|held| held == action) {
                    requested.push(action.to_owned());
                }
            }
        }
        return (!requested.is_empty()).then_some(requested);
    }
    let requested = entry["detail"]["vwBrowserAction"].as_str().or_else(|| {
        let wire = input["action"].as_str()?;
        if builtin {
            vw_agent_tools::browser_input::actions()
                .get(wire)
                .map(String::as_str)
        } else {
            Some(wire)
        }
    });
    requested
        .filter(|action| actions.contains(action))
        .map(|action| vec![action.to_owned()])
}

pub(super) fn scope(entry: &Value, tool: &str, tier: u64) -> Scopeable {
    let input = &entry["detail"]["effect"]["input"];
    let builtin = tool == "mcp_vw.browser_act";
    if !builtin && tool != "mcp_vw.chrome_execute" {
        return Scopeable::No;
    }
    if !builtin && input["sessionId"].as_str().is_none_or(str::is_empty) {
        return Scopeable::No;
    }
    let tier = match tier {
        1 => Tier::AcceptEdits,
        2 => Tier::FullAccess,
        _ => return Scopeable::No,
    };
    let browser = "a browser this chat uses";
    let Ok(texts) = serde_json::from_str::<Value>(vw_ask::chrome::ACTION_TEXT) else {
        return Scopeable::No;
    };
    let actions = if builtin {
        let Some(actions) = crate::chrome_control::builtin::package_actions(tier) else {
            return Scopeable::No;
        };
        actions
    } else {
        vw_ask::chrome::package_actions(tier).collect()
    };
    let Some(requested) = requested_actions(entry, input, builtin, tier, &actions) else {
        return Scopeable::No;
    };
    let descriptions: Vec<_> = actions
        .iter()
        .filter_map(|action| {
            texts["actions"][action]
                .as_str()
                .filter(|text| !text.trim().is_empty())
        })
        .map(|text| {
            text.replace(" on {page}", "")
                .replace(" onto {page}", "")
                .replace(" to {page}", "")
                .replace(" for {page}", "")
                .replace(" from {page}", "")
                .replace("{page}'s ", "")
                .replace("{in}", &format!(" in {browser}"))
                .replace("{your}", "the browser's")
                .replace("{submit}", "")
                .replace("{key}", "a key")
        })
        .collect();
    if descriptions.is_empty() || descriptions.len() != actions.len() {
        return Scopeable::No;
    }
    Scopeable::Grant {
        tool: tool.to_owned(),
        permissions: vec![GrantPermission {
            scope: SCOPE.into(),
            session_pattern: json!({"tier":tier as u8,"actions":actions,"requested":requested})
                .to_string(),
            exact: true,
        }],
        labels: descriptions.clone(),
        covers: vec![CoversPart::words(&format!(
            "For this chat, in every browser you use: {}",
            descriptions.join("; ")
        ))],
    }
}

pub(super) fn covers(
    grant: &SessionGrant,
    session: &str,
    tool: &str,
    wanted: &[GrantPermission],
) -> bool {
    if grant.session != session
        || !matches!(
            grant.tool.as_str(),
            "mcp_vw.browser_act" | "mcp_vw.chrome_execute"
        )
        || !matches!(tool, "mcp_vw.browser_act" | "mcp_vw.chrome_execute")
        || grant.permissions.len() != 1
        || wanted.len() != 1
    {
        return false;
    }
    let parse = |permission: &GrantPermission| {
        (permission.scope == SCOPE && permission.exact)
            .then(|| serde_json::from_str::<Value>(&permission.session_pattern).ok())
            .flatten()
    };
    let (Some(have), Some(want)) = (parse(&grant.permissions[0]), parse(&wanted[0])) else {
        return false;
    };
    have["actions"]
        .as_array()
        .zip(want["requested"].as_array())
        .zip(want["actions"].as_array())
        .is_some_and(|((have, requested), allowed)| {
            !requested.is_empty()
                && requested.len() <= 16
                && requested.iter().all(|action| {
                    action.as_str().is_some_and(|action| !action.is_empty())
                        && have.contains(action)
                        && allowed.contains(action)
                })
        })
        && have["tier"]
            .as_u64()
            .zip(want["tier"].as_u64())
            .is_some_and(|(have, want)| {
                have >= want && (1..=2).contains(&have) && (1..=2).contains(&want)
            })
}
