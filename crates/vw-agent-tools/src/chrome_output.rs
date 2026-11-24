//! App-owned output options layered over the generated helper catalog; used by local MCP discovery.
//! The gateway consumes these fields, so older helpers and extensions receive their existing arguments.
use serde_json::{Value, json};

pub(crate) const DESCRIPTION: &str = " An inline page.read defaults to 32 controls, 10000 text characters and a 10000-byte data budget; returned cursors continue captured content. Allow one full-size read per code-mode return; choose a smaller maxBytes when combining reads. For a single page.read, saveTo:\"file\" explicitly saves that read result as JSON in the chat folder, including remaining-content cursors. It does not fetch remaining pages. Omitted saveTo returns the read inline without creating a file.";

pub(crate) fn catalog() -> Vec<Value> {
    let mut tools = crate::chrome_specs::tool_list();
    for tool in &mut tools {
        if tool["name"] == "chrome_execute" {
            let description = tool["description"].as_str().unwrap_or_default();
            tool["description"] = json!(format!("{description}{DESCRIPTION}"));
            if let Some(properties) = tool["inputSchema"]["properties"].as_object_mut() {
                properties.extend(crate::browser_data::properties(true));
            }
            tool["description"] = json!(format!(
                "{}{}",
                tool["description"].as_str().unwrap_or_default(),
                crate::browser_data::DESCRIPTION
            ));
        } else if tool["name"] == "chrome_session" {
            let description = tool["description"].as_str().unwrap_or_default();
            tool["description"] = json!(format!(
                "{description} close_browser requires the exact profileId for this chat's external browser. It closes only the app-owned Firefox window; in Chrome, Edge or Safari it closes the tabs this chat opened in its own windows, then ends this chat's session, and leaves the person's browser process, windows and tabs alone. Use it when Stop cannot be confirmed or before switching browsers from a stuck session. Ask mode shows an approval card."
            ));
            if let Some(actions) =
                tool["inputSchema"]["properties"]["action"]["enum"].as_array_mut()
            {
                actions.push(json!("close_browser"));
            }
        }
    }
    tools
}

#[cfg(test)]
mod tests {
    #[test]
    fn session_catalog_advertises_close_browser_and_its_scope() {
        let session = super::catalog()
            .into_iter()
            .find(|tool| tool["name"] == "chrome_session")
            .unwrap();
        assert!(
            session["inputSchema"]["properties"]["action"]["enum"]
                .as_array()
                .unwrap()
                .iter()
                .any(|action| action == "close_browser")
        );
        let description = session["description"].as_str().unwrap();
        assert!(description.contains("profileId"), "{description}");
        assert!(description.contains("person's browser"), "{description}");
    }

    #[test]
    fn browser_read_description_matches_inline_defaults() {
        let description = super::catalog()
            .into_iter()
            .find(|tool| tool["name"] == "chrome_execute")
            .unwrap()["description"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(description.contains("32 controls"), "{description}");
        assert!(description.contains("10000-byte"), "{description}");
        assert!(!description.contains("eight controls"), "{description}");
        assert!(description.contains("one full-size read"), "{description}");
        assert!(description.contains("smaller maxBytes"), "{description}");
    }
}
