//! Firefox page sharing (Ask Vibe Work through the signed add-on) is a person's action: Settings, agent facts,
//! setup and Connect lead to the add-on, and a shared page reaches the composer without giving the agent the
//! person's Firefox. Uses scripted owners and a recording Firefox launcher.
//!
//! PREDICT (before the change): sharing was hidden (`PAGE_SHARING_AVAILABLE` false), so Settings and agent facts
//! had no sharing row, Connect was refused with "not available in this version", setup never opened the add-on
//! prompt, and a shared Firefox page would have switched the chat to the person's Firefox and bound its window.
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
    core_on(name, keys, registration, None).await
}

/// [`core`] with the setup's OS shape (`Some(true)`: macOS folders, with a Firefox app bundle in the home).
async fn core_on(
    name: &str,
    keys: Option<Arc<vw_keys::KeyService>>,
    registration: &registry_native_tests::TestRoot,
    macos: Option<bool>,
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
        config.chrome_control.macos = macos;
        if macos == Some(true) {
            let firefox = dir.join("home/Applications/Firefox.app/Contents/MacOS/firefox");
            std::fs::create_dir_all(firefox.parent().unwrap()).unwrap();
            std::fs::write(firefox, b"browser").unwrap();
            std::fs::create_dir_all(dir.join("home/Library/Application Support/Firefox/Profiles"))
                .unwrap();
        }
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
async fn settings_and_agent_facts_show_sharing_as_the_persons_action() {
    let registration = registry_native_tests::TestRoot::new("firefox-sharing-facts");
    let (core, _) = core("firefox-sharing-facts", None, &registration).await;
    core.shared
        .chrome_control
        .set_browser_enabled(agent_browser::Browser::Firefox, true)
        .unwrap();
    core.shared
        .chrome_control
        .owner
        .connected_profiles
        .observe(&json!({"profiles":[
            {"profileId":"firefox:one","connected":true,"extensionVersion":"0.1.7"}
        ]}));
    let view = commands::view(&core.shared);
    let sharing = view
        .firefox_sharing
        .expect("the Ask Vibe Work row is shown");
    assert_eq!(sharing.state, AddonState::Connected);
    assert!(view.firefox.is_some(), "the owned window stays");
    let firefox = firefox_fact(&core);
    let fact = firefox["sharing"].to_string();
    assert!(fact.contains("Ask Vibe Work"), "{fact}");
    assert!(fact.contains("firefox:agent"), "{fact}");
    assert!(fact.contains("cannot"), "{fact}");
    core.shutdown().await;
}

#[tokio::test]
async fn connect_opens_firefox_add_ons() {
    let registration = registry_native_tests::TestRoot::new("firefox-sharing-store");
    let (core, opened) = core("firefox-sharing-store", None, &registration).await;
    core.chrome_control(crate::Origin::Ui, ChromeControlRequest::OpenFirefoxStore {})
        .await
        .unwrap();
    assert_eq!(*opened.lock().unwrap(), ["about:addons"]);
    core.shutdown().await;
}

#[tokio::test]
async fn setup_registers_firefox_and_opens_the_add_on_prompt() {
    let root = std::env::var_os("VW_TEST_ROOT")
        .map_or_else(std::env::temp_dir, std::path::PathBuf::from)
        .join(format!("firefox-sharing-setup-{}", std::process::id()));
    let keys = owner_credential_tests::memory_keys(&root).await;
    let registration = registry_native_tests::TestRoot::new("firefox-sharing-setup");
    let (core, opened) = core("firefox-sharing-setup", Some(keys.clone()), &registration).await;
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
    assert!(
        offer.runs.contains("Opens Firefox Add-ons"),
        "{}",
        offer.runs
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
    assert!(view.firefox_sharing.is_some());
    let chrome = &core.shared.chrome_control;
    assert!(setup::registered(
        &chrome.config,
        chrome.dir(),
        "Mozilla Firefox"
    ));
    assert_eq!(*opened.lock().unwrap(), ["about:addons"]);
    core.shutdown().await;
    keys.shutdown().await;
}

/// Connect on a set-up Firefox whose host manifest the person removed (macOS: the whole Mozilla folder gone, the
/// add-on removed): the setup's own manifest is written again before Firefox Add-ons opens, so the re-added add-on
/// finds the helper. PREDICT (before the fix): Connect (`OpenFirefoxStore`) only opened Add-ons, and on macOS
/// nothing else repairs a registration, so the manifest stayed missing and the add-on said "Native host
/// disconnected".
#[cfg(unix)]
#[tokio::test]
async fn connect_writes_a_missing_firefox_manifest_of_this_setup_again() {
    let root = std::env::var_os("VW_TEST_ROOT")
        .map_or_else(std::env::temp_dir, std::path::PathBuf::from)
        .join(format!("firefox-sharing-repair-{}", std::process::id()));
    let keys = owner_credential_tests::memory_keys(&root).await;
    let registration = registry_native_tests::TestRoot::new("firefox-sharing-repair");
    let (core, opened) = core_on(
        "firefox-sharing-repair",
        Some(keys.clone()),
        &registration,
        Some(true),
    )
    .await;
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
    core.chrome_control(
        crate::Origin::Ui,
        ChromeControlRequest::SetupFirefox {
            offer_id: offer.offer_id,
        },
    )
    .await
    .unwrap();
    let home = core.shared.chrome_control.config.user_home.clone().unwrap();
    let mozilla = home.join("Library/Application Support/Mozilla");
    let manifest = mozilla
        .join("NativeMessagingHosts")
        .join(format!("{}.json", setup::HOST_NAME));
    let written = std::fs::read(&manifest).expect("setup wrote the Firefox manifest");
    std::fs::remove_dir_all(&mozilla).unwrap();
    core.chrome_control(crate::Origin::Ui, ChromeControlRequest::OpenFirefoxStore {})
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(&manifest).ok(),
        Some(String::from_utf8(written).unwrap()),
        "Connect leaves the Firefox manifest missing"
    );
    assert_eq!(
        opened.lock().unwrap().last().map(String::as_str),
        Some("about:addons")
    );
    core.shutdown().await;
    keys.shutdown().await;
}

#[tokio::test]
async fn a_shared_firefox_page_reaches_the_chat_without_giving_the_agent_the_persons_firefox() {
    let registration = registry_native_tests::TestRoot::new("firefox-sharing-claim");
    let (core, _) = core("firefox-sharing-claim", None, &registration).await;
    let _ = core
        .shared
        .store
        .load(
            json!({"session":{"id":"chat"},"history":[],"turns":[]}),
            vw_session::LoadKind::New,
        )
        .unwrap();
    core.shared
        .chrome_control
        .set_browser_enabled(agent_browser::Browser::Firefox, true)
        .unwrap();
    let calls = test_support::answering(
        &core.shared,
        |_, _| json!({"structuredContent":{"requestId":"read","status":"completed","data":{"url":"https://page.test/","title":"Shared page","text":"Selected words"}}}),
    );
    owner::observe(
        &Arc::downgrade(&core.shared),
        &json!({"method":"notifications/client/share","params":{"shareId":"page","share":{"kind":"tab","pageUrl":"https://page.test/","pageTitle":"Shared page","clientSessionId":"chat","handover":{"profileId":"firefox:one","browserSessionId":"browser","tab":{"tabId":7,"windowId":9,"documentId":"document","url":"https://page.test/","title":"Shared page","origin":"https://page.test","active":true}}}}}),
    )
    .unwrap();
    let crate::CommandResult::ChromeSelection {
        selection: Some(item),
    } = core
        .execute(
            crate::Origin::Ui,
            serde_json::from_value(
                json!({"type":"chromeControl","request":{"op":"takeSelection","selectionId":"page"}}),
            )
            .unwrap(),
        )
        .await
        .unwrap()
    else {
        panic!("Missing Firefox page context")
    };
    assert_eq!(serde_json::to_value(&item).unwrap()["kind"], "page");
    assert_eq!(item.session_id.as_deref(), Some("chat"));
    assert_eq!(item.text, "Selected words");
    assert!(item.model_text.contains("untrustedPageContent"));
    assert!(
        !item.model_text.contains("browserTab"),
        "{}",
        item.model_text
    );
    let calls = calls.lock().unwrap().clone();
    assert_eq!(
        calls.len(),
        1,
        "the shared page is read once for the composer"
    );
    for (_, args) in &calls {
        assert!(args["_tabContext"]["clientSessionId"].is_null(), "{args}");
        assert_eq!(args["_tabContext"]["profileId"], "firefox:one");
    }
    assert_ne!(
        profile_bindings::selected(&core.shared.chrome_control, "chat").as_deref(),
        Some("firefox:one"),
        "the chat keeps its own browser"
    );
    core.shutdown().await;
}

// PREDICT (before the core check): the call went to the helper, which is the only other place that refuses it.
#[tokio::test]
async fn agent_calls_naming_the_persons_firefox_are_refused_before_the_helper() {
    let registration = registry_native_tests::TestRoot::new("firefox-sharing-agent");
    let (core, _) = core("firefox-sharing-agent", None, &registration).await;
    let _ = core
        .shared
        .store
        .load(
            json!({"session":{"id":"chat"},"history":[],"turns":[]}),
            vw_session::LoadKind::New,
        )
        .unwrap();
    core.shared.access.observe_request(
        "session/agent/update",
        "chat",
        &json!({"agentName":"auto-approve"}),
    );
    core.shared
        .chrome_control
        .set_browser_enabled(agent_browser::Browser::Firefox, true)
        .unwrap();
    let calls = test_support::answering(&core.shared, |_, _| json!({"structuredContent":{}}));
    for (tool, args) in [
        (
            "chrome_session",
            json!({"action":"open","profileId":"firefox:one"}),
        ),
        (
            "chrome_session",
            json!({"action":"open","profileId":"firefox:one","tabId":7}),
        ),
        (
            "chrome_status",
            json!({"profileId":"firefox:one","action":"tabs.list"}),
        ),
    ] {
        let error = gateway::call(&core.shared, "chat", tool, args.clone())
            .await
            .unwrap_err();
        assert!(error.contains("Ask Vibe Work"), "{tool} {args}: {error}");
        assert!(error.contains("firefox:agent"), "{error}");
    }
    assert!(
        calls.lock().unwrap().is_empty(),
        "nothing reached the helper"
    );
    assert!(
        firefox_sharing::refuse_agent_reach(
            "chrome_session",
            &json!({"action":"open","profileId":"FIREFOX:One"})
        )
        .is_err()
    );
    assert!(
        firefox_sharing::refuse_agent_reach(
            "chrome_session",
            &json!({"profileId":"firefox:agent"})
        )
        .is_ok()
    );
    assert!(
        firefox_sharing::refuse_agent_reach("chrome_status", &json!({"profileId":"firefox:one"}))
            .is_ok()
    );
    core.shutdown().await;
}
