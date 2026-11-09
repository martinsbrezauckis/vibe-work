//! Runtime module boundaries through the same core views and commands used by the app.
//! Uses the offline core; no browser, native dialog, helper or provider is started.
use crate::{ChromeControlRequest, CommandResult, Origin};

#[tokio::test]
async fn browser_settings_follow_the_compiled_modules() {
    let core = crate::offline_core::start("release-browser-modules").await;
    let CommandResult::ChromeControl { view, .. } = core
        .chrome_control(Origin::Ui, ChromeControlRequest::Read {})
        .await
        .unwrap()
    else {
        panic!("browser settings view");
    };
    assert_eq!(view.edge.is_some(), cfg!(feature = "your-edge"));
    assert_eq!(view.firefox.is_some(), cfg!(feature = "your-firefox"));
    core.shutdown().await;
}

#[cfg(not(feature = "your-firefox"))]
#[tokio::test]
async fn disabled_firefox_has_no_settings_or_usable_commands_even_with_a_browser_package() {
    let core = crate::offline_core::start_with("release-firefox-off", |config, dir| {
        config.chrome_control.package = Some(dir.join("package"));
    })
    .await;
    for request in [
        ChromeControlRequest::OfferFirefoxSetup {},
        ChromeControlRequest::SetupFirefox {
            offer_id: "unused".into(),
        },
        ChromeControlRequest::OpenFirefoxStore {},
        ChromeControlRequest::OpenFirefox {},
        ChromeControlRequest::CloseFirefox {},
    ] {
        assert!(core.chrome_control(Origin::Ui, request).await.is_err());
    }
    core.shutdown().await;
}

#[cfg(not(feature = "feedback"))]
#[tokio::test]
async fn disabled_feedback_refuses_every_entry_point() {
    let core = crate::offline_core::start("release-feedback-off").await;
    for origin in [Origin::Ui, Origin::DebugApi] {
        assert!(
            core.feedback(origin, crate::FeedbackRequest::Read)
                .await
                .is_err()
        );
    }
    assert!(!vw_agent_tools::wire::tools::SESSION_TOOLS.contains(&"feedback_draft"));
    core.shutdown().await;
}

#[test]
fn video_frames_is_listed_only_on_supported_platforms() {
    let tools = vw_agent_tools::mcp::tool_list();
    let listed = tools
        .as_array()
        .unwrap()
        .iter()
        .any(|tool| tool["name"] == "video_frames");
    assert_eq!(
        listed,
        cfg!(all(
            feature = "video",
            any(
                windows,
                target_os = "macos",
                all(target_os = "linux", feature = "linux-video")
            )
        ))
    );
}

#[tokio::test]
async fn safari_settings_and_agent_facts_follow_the_off_by_default_module() {
    let core = crate::offline_core::start("release-safari-module").await;
    let result = core
        .chrome_control(Origin::Ui, ChromeControlRequest::Read {})
        .await
        .unwrap();
    let value = serde_json::to_value(result).unwrap();
    assert_eq!(
        value["view"]["safari"].is_object(),
        cfg!(feature = "your-safari")
    );
    let addons = crate::chrome_control::installation::addons(&core.shared);
    assert_eq!(
        addons
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["id"] == "safari"),
        cfg!(feature = "your-safari")
    );
    core.shutdown().await;
}
