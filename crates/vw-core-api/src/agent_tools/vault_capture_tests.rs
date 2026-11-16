//! Agent saves to the vault in the default build: the capture tools are advertised and a page in the person's own
//! browser is refused with a next step the agent can follow.
//!
//! PREDICT: before the capture became part of every build, the default build advertised only the refusing
//! `browser_capture_secret` stub and no `secret_capture_command`, its policy list had no `vault_save`, the working
//! note had no capture line, and a Chrome/Edge/Safari/Firefox source got the generic "saving a key to the vault is
//! the user's action" refusal instead of a browser-named next step. Both tests fail on that build.
use serde_json::{Value, json};
use vw_agent_tools::wire::tools as names;

fn advertised(name: &str) -> Option<Value> {
    vw_agent_tools::mcp::tool_list()
        .as_array()
        .and_then(|tools| tools.iter().find(|t| t["name"] == name).cloned())
}

#[test]
fn capture_tools_are_advertised_in_default_build() {
    let command = advertised(names::SECRET_CAPTURE_COMMAND).expect("command capture advertised");
    assert_eq!(
        command["inputSchema"]["required"],
        json!(["name", "program"])
    );
    let browser = advertised(names::BROWSER_CAPTURE_SECRET).expect("browser capture advertised");
    assert_eq!(browser["inputSchema"]["required"], json!(["ref", "name"]));
    let description = browser["description"].as_str().unwrap_or_default();
    assert!(!description.contains("always refuses"), "{description}");
    assert!(names::SESSION_TOOLS.contains(&names::SECRET_CAPTURE_COMMAND));
    assert!(names::SECRET_TOOLS.contains(&names::SECRET_CAPTURE_COMMAND));
    assert!(vw_ask::policies().any(|p| p == "vault_save"));
    assert!(crate::working_note::vault_capture_line().is_some());
}

#[tokio::test]
async fn chrome_ref_capture_refused_with_next_step() {
    let core = crate::offline_core::start_with("vault-capture-source", |_, _| {}).await;
    for (target, browser) in [
        ("chrome", vw_vocabulary::texts::CHROME_NAME),
        ("edge", vw_vocabulary::texts::EDGE_NAME),
        ("safari", vw_vocabulary::texts::SAFARI_NAME),
        ("firefox", vw_vocabulary::texts::FIREFOX_NAME),
    ] {
        let args = json!({"ref":"e00000001","name":"prod_deploy","target":target});
        let reason =
            super::browser_tools::call(&core.shared, "s", names::BROWSER_CAPTURE_SECRET, args)
                .await
                .unwrap_err();
        assert_eq!(
            reason,
            format!(
                "Not run: saving from {browser} is not available yet. Ask the user to open the page in Vibe Work's browser panel, or save it from a command's output."
            )
        );
    }
    core.shutdown().await;
}
