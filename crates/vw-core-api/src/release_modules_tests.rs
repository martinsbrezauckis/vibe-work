//! Runtime module boundaries through the same core views and commands used by the app.
//! Uses the offline core; no browser, native dialog, helper or provider is started.
use crate::{ChromeControlRequest, Origin};

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
