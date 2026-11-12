//! Firefox page sharing is hidden in 0.1.10 (`firefox_sharing::PAGE_SHARING_AVAILABLE`): Settings, agent facts, setup
//! and the add-on request never lead to the add-on. Uses scripted owners and a recording Firefox launcher.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "fixture folders and the Firefox fact must exist for the test to mean anything"
)]
use super::*;
use std::sync::{Arc, Mutex};

type Opened = Arc<Mutex<Vec<String>>>;

async fn core(
    name: &str,
    keys: Option<Arc<vw_keys::KeyService>>,
    registration: &registry_native_tests::TestRoot,
) -> (crate::Core, Opened) {
    let opened = Opened::default();
    let record = opened.clone();
    let core = crate::offline_core::start_with(name, |config, dir| {
        let package = dir.join("package");
        std::fs::create_dir_all(package.join("bin")).unwrap();
        std::fs::write(package.join("bin").join(setup::helper_name()), b"helper").unwrap();
        config.keys = keys;
        config.chrome_control.package = Some(package);
        config.chrome_control.user_home = Some(dir.join("home"));
        config.chrome_control.firefox_registry_root =
            format!(r"{}\Mozilla\NativeMessagingHosts", registration.key());
        config.chrome_control.firefox_launcher = Some(Arc::new(move |url: &str| {
            record.lock().unwrap().push(url.to_owned());
            Ok(())
        }));
    })
    .await;
    (core, opened)
}

fn firefox_fact(core: &crate::Core) -> serde_json::Value {
    installation::addons(&core.shared)
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["id"] == "firefox")
        .cloned()
        .expect("Firefox facts while its module is on")
}

#[tokio::test]
async fn settings_and_agent_facts_carry_no_page_sharing_even_for_an_outdated_add_on() {
    let registration = registry_native_tests::TestRoot::new("firefox-hidden-facts");
    let (core, _) = core("firefox-hidden-facts", None, &registration).await;
    // An outdated live add-on used to ask the person to click Connect; it stays out of every view now.
    core.shared
        .chrome_control
        .owner
        .connected_profiles
        .observe(&json!({"profiles":[
            {"profileId":"firefox:one","connected":true,"extensionVersion":"0.1.6"}
        ]}));
    let view = commands::view(&core.shared);
    assert!(view.firefox_sharing.is_none());
    assert!(view.firefox.is_some(), "the owned window stays");
    let firefox = firefox_fact(&core);
    assert!(firefox.get("sharing").is_none(), "{firefox}");
    let text = firefox.to_string();
    let connect = vw_vocabulary::text("connectorConnect").unwrap();
    for hidden in ["Ask Vibe Work", "add-on", connect] {
        assert!(!text.contains(hidden), "{hidden}: {text}");
    }
    assert!(text.contains("Open window"), "{text}");
    core.shutdown().await;
}

#[tokio::test]
async fn the_add_on_request_is_refused_before_firefox_starts() {
    let registration = registry_native_tests::TestRoot::new("firefox-hidden-store");
    let (core, opened) = core("firefox-hidden-store", None, &registration).await;
    let error = core
        .chrome_control(crate::Origin::Ui, ChromeControlRequest::OpenFirefoxStore {})
        .await
        .unwrap_err();
    assert_eq!(error.message, firefox_sharing::page_sharing_unavailable());
    assert!(error.message.contains("Open window"), "{}", error.message);
    assert!(opened.lock().unwrap().is_empty());
    core.shutdown().await;
}

#[tokio::test]
async fn setup_registers_firefox_without_opening_the_add_on_prompt() {
    let root = std::env::var_os("VW_TEST_ROOT")
        .map_or_else(std::env::temp_dir, std::path::PathBuf::from)
        .join(format!("firefox-hidden-setup-{}", std::process::id()));
    let keys = owner_credential_tests::memory_keys(&root).await;
    let registration = registry_native_tests::TestRoot::new("firefox-hidden-setup");
    let (core, opened) = core("firefox-hidden-setup", Some(keys.clone()), &registration).await;
    core.shared
        .chrome_control
        .owner
        .answering(|_, _| json!({"applied":true}));
    let crate::CommandResult::ChromeControl {
        offer: Some(offer), ..
    } = core
        .chrome_control(
            crate::Origin::Ui,
            ChromeControlRequest::OfferFirefoxSetup {},
        )
        .await
        .unwrap()
    else {
        panic!("Firefox setup offer")
    };
    assert_eq!(
        offer.runs,
        "Adds a small program to Vibe Work's folder for your user and tells Firefox where it is."
    );
    let crate::CommandResult::ChromeControl { view, .. } = core
        .chrome_control(
            crate::Origin::Ui,
            ChromeControlRequest::SetupFirefox {
                offer_id: offer.offer_id,
            },
        )
        .await
        .unwrap()
    else {
        panic!("Firefox setup result")
    };
    assert!(view.firefox_sharing.is_none());
    let chrome = &core.shared.chrome_control;
    assert!(setup::registered(
        &chrome.config,
        chrome.dir(),
        "Mozilla Firefox"
    ));
    assert!(
        opened.lock().unwrap().is_empty(),
        "setup never opens the add-on prompt"
    );
    core.shutdown().await;
    keys.shutdown().await;
}
