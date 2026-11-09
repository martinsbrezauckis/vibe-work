//! Browser session grants bind the owning tool, browser lease and tier. Descriptions reuse the card's verbs.
//! Called only for core-created Ask cards; native vault grants remain independent.
use super::grants::Scopeable;
use serde_json::{Value, json};
use vw_ask::{CoversPart, GrantPermission, SessionGrant, chrome::Tier};

const SCOPE: &str = "vibe_work_browser";

pub(super) fn scope(entry: &Value, tool: &str, tier: u64) -> Scopeable {
    let input = &entry["detail"]["effect"]["input"];
    let builtin = tool == "mcp_vw.browser_act";
    if !builtin && tool != "mcp_vw.chrome_execute" {
        return Scopeable::No;
    }
    let browser_session = if builtin {
        Value::Null
    } else {
        let Some(id) = input["sessionId"].as_str().filter(|id| !id.is_empty()) else {
            return Scopeable::No;
        };
        json!(id)
    };
    let tier = match tier {
        1 => Tier::AcceptEdits,
        2 => Tier::FullAccess,
        _ => return Scopeable::No,
    };
    let browser = if builtin {
        "the built-in browser"
    } else {
        match entry["vwBrowser"].as_str() {
            Some(vw_vocabulary::texts::CHROME_NAME) => vw_vocabulary::texts::CHROME_NAME,
            Some(vw_vocabulary::texts::EDGE_NAME) => vw_vocabulary::texts::EDGE_NAME,
            Some(vw_vocabulary::texts::SAFARI_NAME) => vw_vocabulary::texts::SAFARI_NAME,
            Some(vw_vocabulary::texts::FIREFOX_NAME) => vw_vocabulary::texts::FIREFOX_NAME,
            _ => "your browser",
        }
    };
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
    if requested.is_none_or(|action| !actions.contains(&action)) {
        return Scopeable::No;
    }
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
                .replace("{your}", &browser.replace("Your ", "your "))
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
            session_pattern:
                json!({"tier":tier as u8,"browserSession":browser_session,"actions":actions})
                    .to_string(),
            exact: true,
        }],
        labels: descriptions.clone(),
        covers: vec![CoversPart::words(&descriptions.join("; "))],
    }
}

pub(super) fn covers(
    grant: &SessionGrant,
    session: &str,
    tool: &str,
    wanted: &[GrantPermission],
) -> bool {
    if grant.session != session
        || grant.tool != tool
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
    have["browserSession"] == want["browserSession"]
        && have["actions"]
            .as_array()
            .zip(want["actions"].as_array())
            .is_some_and(|(have, want)| {
                !want.is_empty() && want.iter().all(|action| have.contains(action))
            })
        && have["tier"]
            .as_u64()
            .zip(want["tier"].as_u64())
            .is_some_and(|(have, want)| {
                have >= want && (1..=2).contains(&have) && (1..=2).contains(&want)
            })
}
