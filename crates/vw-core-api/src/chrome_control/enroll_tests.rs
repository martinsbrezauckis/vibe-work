//! Automatic pairing from the app's Connect: who may mint, revoke order, the one retry, stale attempts and the URL.
//! Uses an offline core, a disposable package and home, a recording launcher and a scripted owner; never opens
//! Chrome or a keyring.
use super::super::addon::{AddonAction, AddonState, AddonView};
use super::super::*;
use crate::Origin;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

#[path = "enroll_log_tests.rs"]
mod decision_logs;

#[path = "enroll_fallback_tests.rs"]
mod fallbacks;

#[path = "enroll_scoped_tests.rs"]
mod scoped_profiles;

#[path = "enroll_timing_tests.rs"]
mod timing;

const OURS: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const PROFILE: &str = "0b5f6a1e-27c4-4c1e-9d3a-6f1e2b3c4d5e";
const TICKET: &str = "AbCdEfGhIjKlMnOpQrStUvWxYz0123456789-_AbCdE";

struct Rig {
    core: crate::Core,
    opened: Arc<Mutex<Vec<String>>>,
    edge_opened: Arc<Mutex<Vec<String>>>,
    calls: Arc<Mutex<Vec<(String, Value)>>>,
    /// Connect writes Chrome's external-extension key: on Windows it goes below this test's own disposable key,
    /// never Chrome's real one, and parallel tests never share it. Dropped after the core.
    _registry: super::super::registry_native_tests::TestRoot,
}

impl Rig {
    async fn new(name: &str, answer: impl Fn(&str, &Value) -> Value + Send + 'static) -> Self {
        let opened = Arc::new(Mutex::new(Vec::new()));
        let launches = opened.clone();
        let edge_opened = Arc::new(Mutex::new(Vec::new()));
        let edge_launches = edge_opened.clone();
        let registry =
            super::super::registry_native_tests::TestRoot::new(&format!("enroll-{name}"));
        let root = format!(r"{}\NativeMessagingHosts", registry.key());
        let edge_root = format!(r"{}\Edge\NativeMessagingHosts", registry.key());
        let core = crate::offline_core::start_with(name, move |config, dir| {
            config.chrome_control.windows_registry_root = root;
            config.chrome_control.edge_registry_root = edge_root;
            config.chrome_control.package = Some(dir.join("package"));
            config.chrome_control.user_home = Some(dir.join("home"));
            config.chrome_control.launcher = Some(Arc::new(move |url| {
                launches.lock().unwrap().push(url.to_owned());
                Ok(())
            }));
            config.chrome_control.edge_launcher = Some(Arc::new(move |url| {
                edge_launches.lock().unwrap().push(url.to_owned());
                Ok(())
            }));
        })
        .await;
        let chrome = &core.shared.chrome_control;
        let package = chrome.config.package.as_ref().unwrap();
        std::fs::create_dir_all(package.join("bin")).unwrap();
        std::fs::write(
            package.join("bin").join(setup::helper_name()),
            b"test helper",
        )
        .unwrap();
        let dir = chrome.dir().unwrap();
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(
            dir.join("setup.json"),
            json!({"files":[],"browsers":["Google Chrome"],"userData":[chrome.config.user_home.as_ref().unwrap().join("chrome-data")]})
                .to_string(),
        )
        .unwrap();
        let calls = chrome.owner.answering(answer);
        // A helper connection is up (its tool list arrived).
        chrome
            .owner
            .tools
            .lock()
            .unwrap()
            .push(json!({"name":"chrome_status"}));
        Self {
            core,
            opened,
            edge_opened,
            calls,
            _registry: registry,
        }
    }

    fn waiting(&self, enroll: bool) {
        let mut params = json!({"profileId":PROFILE,"state":"waiting","fingerprint":OURS});
        if enroll {
            params["enroll"] = json!(1);
            params["extensionVersion"] = json!("0.1.4");
        }
        owner::observe(
            &Arc::downgrade(&self.core.shared),
            &json!({"method":"notifications/owner/pairing","params":params}),
        )
        .unwrap();
    }

    async fn connect(&self, origin: Origin) {
        self.core
            .chrome_control(origin, ChromeControlRequest::Connect {})
            .await
            .unwrap();
    }

    fn methods(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .map(|(name, _)| name.clone())
            .filter(|name| name.starts_with("owner.enroll"))
            .collect()
    }

    fn opened(&self) -> Vec<String> {
        self.opened.lock().unwrap().clone()
    }

    async fn settle(&self) {
        for _ in 0..20 {
            tokio::task::yield_now().await;
        }
    }
}

fn ticket_answer(name: &str, _: &Value) -> Value {
    if name == enroll::MINT {
        json!({"ticket":TICKET,"profileId":PROFILE,"fingerprint":OURS})
    } else {
        json!({})
    }
}

fn plain() -> String {
    format!("chrome-extension://{}/ui.html", setup::EXTENSION_ID)
}

fn expired_url() -> String {
    format!("{}#vw-enroll-reason=expired", plain())
}

fn enroll_url() -> String {
    format!(
        "chrome-extension://{}/ui.html#vw-enroll={TICKET}&p={PROFILE}&f={OURS}",
        setup::EXTENSION_ID
    )
}

#[tokio::test]
async fn connect_ui_mints_debug_api_does_not() {
    let rig = Rig::new("chrome-enroll-origin", ticket_answer).await;
    rig.waiting(true);
    rig.connect(Origin::DebugApi).await;
    assert!(
        !rig.methods().iter().any(|m| m == enroll::MINT),
        "a Debug API Connect asked for a ticket: {:?}",
        rig.methods()
    );
    assert_eq!(rig.opened(), [plain()]);
    rig.waiting(true);
    rig.connect(Origin::Ui).await;
    assert_eq!(rig.opened(), [plain(), enroll_url()]);
    rig.core.shutdown().await;
}

#[tokio::test]
async fn connect_revokes_previous_attempt() {
    let rig = Rig::new("chrome-enroll-revoke", ticket_answer).await;
    rig.waiting(true);
    rig.connect(Origin::Ui).await;
    rig.waiting(true);
    rig.connect(Origin::Ui).await;
    assert_eq!(
        rig.methods(),
        [enroll::REVOKE, enroll::MINT, enroll::REVOKE, enroll::MINT]
    );
    let attempts: Vec<Value> = rig
        .calls
        .lock()
        .unwrap()
        .iter()
        .filter(|(name, _)| name == enroll::MINT)
        .map(|(_, args)| args["attempt"].clone())
        .collect();
    assert_ne!(attempts[0], attempts[1], "each Connect is its own attempt");
    assert_eq!(
        attempts[0].as_str().map(str::len),
        Some(32),
        "16 random bytes"
    );
    // Turning Use my Chrome off revokes too (this helper reported the enrollment fields).
    rig.core
        .chrome_control(
            Origin::Ui,
            ChromeControlRequest::SetEnabled { enabled: false },
        )
        .await
        .unwrap();
    assert_eq!(
        rig.methods().last().map(String::as_str),
        Some(enroll::REVOKE)
    );
    rig.core.shutdown().await;
}

#[tokio::test]
async fn connect_after_earlier_waiting_mints() {
    let rig = Rig::new("chrome-enroll-earlier-waiting", ticket_answer).await;
    // Chrome was waiting before the person clicked: startup alone opens nothing and mints nothing.
    rig.waiting(true);
    rig.settle().await;
    assert_eq!(rig.opened(), Vec::<String>::new());
    assert_eq!(rig.methods(), Vec::<String>::new());
    rig.connect(Origin::Ui).await;
    assert_eq!(rig.opened(), [enroll_url()]);
    rig.core.shutdown().await;
}

#[tokio::test]
async fn connect_before_waiting_mints_once_on_the_next_waiting() {
    let rig = Rig::new("chrome-enroll-later-waiting", ticket_answer).await;
    rig.connect(Origin::Ui).await;
    // No waiting connection yet: the store page opens (install), no mint is asked.
    assert_eq!(rig.methods(), [enroll::REVOKE]);
    rig.waiting(true);
    rig.settle().await;
    assert_eq!(rig.methods(), [enroll::REVOKE, enroll::MINT]);
    assert_eq!(rig.opened().last(), Some(&enroll_url()));
    // A later waiting report (the page reloaded, another profile) never mints again for this Connect.
    rig.waiting(true);
    rig.settle().await;
    assert_eq!(rig.methods(), [enroll::REVOKE, enroll::MINT]);
    rig.core.shutdown().await;
}

#[tokio::test]
async fn no_waiting_retries_once_then_opens_the_pair_page() {
    let rig = Rig::new("chrome-enroll-retry", |name, _| {
        if name == enroll::MINT {
            json!({"none":"no_waiting"})
        } else {
            json!({})
        }
    })
    .await;
    rig.waiting(true);
    rig.connect(Origin::Ui).await;
    assert!(
        rig.opened().is_empty(),
        "no_waiting waits for the next report"
    );
    rig.waiting(true);
    rig.settle().await;
    assert_eq!(rig.methods(), [enroll::REVOKE, enroll::MINT, enroll::MINT]);
    assert_eq!(rig.opened(), [plain()]);
    rig.core.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn stale_attempt_never_mints() {
    let rig = Rig::new("chrome-enroll-stale", ticket_answer).await;
    rig.connect(Origin::Ui).await;
    tokio::time::advance(enroll::ATTEMPT_LIFE + std::time::Duration::from_secs(1)).await;
    rig.waiting(true);
    rig.settle().await;
    assert_eq!(
        rig.methods(),
        [enroll::REVOKE],
        "an expired attempt asked a mint"
    );
    assert_eq!(rig.opened().last(), Some(&expired_url()));
    rig.core.shutdown().await;
}

#[tokio::test]
async fn the_current_store_extension_and_an_older_helper_get_the_pair_page() {
    // The live extension sends no `enroll`: no mint at all.
    let rig = Rig::new("chrome-enroll-store-extension", ticket_answer).await;
    rig.waiting(false);
    rig.connect(Origin::Ui).await;
    assert_eq!(rig.methods(), [enroll::REVOKE]);
    assert_eq!(rig.opened(), [plain()]);
    rig.core.shutdown().await;
    // An older helper answers the enrollment requests with "method not found".
    let rig = Rig::new("chrome-enroll-old-helper", |name, _| {
        if name.starts_with("owner.enroll") {
            json!({"rpcError":{"code":-32601,"message":"Method not found"}})
        } else {
            json!({})
        }
    })
    .await;
    rig.waiting(true);
    rig.connect(Origin::Ui).await;
    assert_eq!(rig.opened(), [plain()]);
    rig.core.shutdown().await;
}

#[tokio::test]
async fn opens_enroll_url_once_without_profile_flag() {
    let rig = Rig::new("chrome-enroll-url", ticket_answer).await;
    rig.waiting(true);
    rig.connect(Origin::Ui).await;
    let opened = rig.opened();
    assert_eq!(
        opened,
        [enroll_url()],
        "exactly one page, with the full fingerprint"
    );
    assert!(opened[0].ends_with(&format!("&f={OURS}")));
    // The real launcher passes the address as Chrome's only argument: no profile directory flag.
    assert_eq!(
        browser::launch_args("chrome-extension://x/ui.html#t"),
        [std::ffi::OsString::from("chrome-extension://x/ui.html#t")]
    );
    // The view and its events never carry the ticket.
    let view = serde_json::to_string(&rig.core.chrome_view()).unwrap();
    assert!(!view.contains(TICKET));
    rig.core.shutdown().await;
}

#[test]
fn mint_answers_are_checked_before_any_page_opens() {
    use super::Minted;
    let minted = |answer: Option<&Value>, expected: Option<&str>| {
        super::minted(answer, expected, enroll_browser::Browser::Chrome)
    };
    let good = json!({"ticket":TICKET,"profileId":PROFILE,"fingerprint":OURS});
    assert!(matches!(
        minted(Some(&good), Some(OURS)),
        Minted::Ticket { .. }
    ));
    assert_eq!(
        minted(Some(&good), Some(&"b".repeat(64))),
        Minted::Other,
        "another owner's fingerprint"
    );
    for bad in [
        json!({"ticket":"short","profileId":PROFILE,"fingerprint":OURS}),
        json!({"ticket":format!("{TICKET}#"),"profileId":PROFILE,"fingerprint":OURS}),
        json!({"ticket":TICKET,"profileId":"../x","fingerprint":OURS}),
        json!({"ticket":TICKET,"profileId":PROFILE,"fingerprint":"AA"}),
        json!({"none":"ambiguous"}),
        json!({"rpcError":{}}),
    ] {
        assert_eq!(minted(Some(&bad), None), Minted::Other, "{bad}");
    }
    assert_eq!(
        minted(Some(&json!({"none":"no_waiting"})), None),
        Minted::NoWaiting
    );
    assert_eq!(minted(None, None), Minted::Other);
}

fn has_attempt(rig: &Rig) -> bool {
    rig.core
        .shared
        .chrome_control
        .owner
        .enroll
        .lock()
        .unwrap()
        .attempt
        .is_some()
}

/// The Debug API can drive the window, whose commands then arrive with the window's origin: no ticket.
#[tokio::test]
async fn debug_api_driven_window_connect_never_mints() {
    let rig = Rig::new("chrome-enroll-driven", ticket_answer).await;
    rig.core.set_window_drive(|| true);
    rig.waiting(true);
    rig.connect(Origin::Ui).await;
    assert!(
        !rig.methods().iter().any(|m| m == enroll::MINT),
        "{:?}",
        rig.methods()
    );
    assert_eq!(rig.opened(), [plain()]);
    rig.core.shutdown().await;
}

/// A window Connect still waiting for Chrome, then a Debug API Connect: the waiting report never mints for the
/// window's earlier attempt.
#[tokio::test]
async fn a_debug_api_connect_never_reuses_a_window_attempt() {
    let rig = Rig::new("chrome-enroll-reuse", ticket_answer).await;
    rig.connect(Origin::Ui).await;
    assert!(has_attempt(&rig));
    rig.connect(Origin::DebugApi).await;
    assert!(!has_attempt(&rig));
    rig.waiting(true);
    rig.settle().await;
    assert!(
        !rig.methods().iter().any(|m| m == enroll::MINT),
        "{:?}",
        rig.methods()
    );
    assert_eq!(rig.opened().last(), Some(&plain()));
    rig.core.shutdown().await;
}

#[tokio::test]
async fn stop_and_disconnect_clear_the_attempt() {
    let rig = Rig::new("chrome-enroll-clear", ticket_answer).await;
    rig.connect(Origin::Ui).await;
    assert!(has_attempt(&rig));
    rig.core
        .chrome_control(
            Origin::Ui,
            ChromeControlRequest::SetEnabled { enabled: false },
        )
        .await
        .unwrap();
    assert!(!has_attempt(&rig), "stopping the owner keeps an attempt");
    let mut enroll = super::Enroll::default();
    enroll.begin(tokio::time::Instant::now());
    enroll.connection_ended(enroll_browser::Browser::Chrome);
    assert!(
        enroll.attempt.is_none(),
        "a helper disconnect keeps an attempt"
    );
    rig.core.shutdown().await;
}

fn status(rig: &Rig, connected: bool) {
    let profiles = if connected {
        json!([{"profileId":PROFILE,"connected":true}])
    } else {
        json!([])
    };
    owner::update_pairing(
        &Arc::downgrade(&rig.core.shared),
        &json!({"structuredContent":{"profiles":profiles,"ownerFingerprint":OURS}}),
    );
}

fn addon(rig: &Rig) -> AddonView {
    rig.core.chrome_view().setup.addon.unwrap()
}

/// Windows walk: the card said Pair in Chrome (with the code) while Chrome still asked to enable the extension. A
/// helper status answer always names our fingerprint; only the helper's waiting report is the pairing step.
#[tokio::test]
async fn the_pairing_step_shows_only_after_a_waiting_report() {
    let rig = Rig::new("chrome-addon-waiting", ticket_answer).await;
    status(&rig, false);
    let before = addon(&rig);
    assert_ne!(before.state, AddonState::NotPaired, "{before:?}");
    assert_ne!(
        rig.core.chrome_view().setup.pairing,
        "Waiting for you to pair in Chrome"
    );
    rig.waiting(false);
    let after = addon(&rig);
    assert_eq!(after.state, AddonState::NotPaired);
    assert_eq!(after.action, Some(AddonAction::OpenPairingPage));
    rig.core.shutdown().await;
}

/// Windows walk: after pairing, closing Chrome turned the card back to Pair in Chrome with the code.
#[tokio::test]
async fn a_paired_setup_with_chrome_closed_never_shows_the_pairing_step() {
    let rig = Rig::new("chrome-addon-closed", ticket_answer).await;
    rig.waiting(false);
    status(&rig, true);
    assert_eq!(addon(&rig).state, AddonState::Connected);
    // Chrome closed: its installed extension remains, but the helper has no connected profile.
    let chrome = &rig.core.shared.chrome_control;
    std::fs::create_dir_all(
        chrome
            .config
            .user_home
            .as_ref()
            .unwrap()
            .join("chrome-data/Default/Extensions")
            .join(&chrome.config.extension_id),
    )
    .unwrap();
    status(&rig, false);
    let closed = addon(&rig);
    assert_eq!(closed.state, AddonState::ChromeClosed);
    assert_eq!(closed.label, "Connected when Chrome is open");
    assert_eq!(closed.action, None);
    rig.core.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn labels_connecting_falls_back_to_pair_after_15_seconds() {
    let rig = Rig::new("chrome-addon-connecting", ticket_answer).await;
    rig.waiting(true);
    rig.connect(Origin::Ui).await;
    assert_eq!(addon(&rig).state, AddonState::Connecting);
    tokio::time::advance(std::time::Duration::from_millis(14_990)).await;
    assert_eq!(addon(&rig).state, AddonState::Connecting);
    tokio::time::advance(std::time::Duration::from_millis(20)).await;
    let fallback = addon(&rig);
    assert_eq!(
        fallback.state,
        AddonState::NotPaired,
        "Pair in Chrome + code"
    );
    assert!(rig.core.chrome_view().setup.fingerprint.is_some());
    rig.core.shutdown().await;
}

/// Windows walk: after Remove the app asked to remove the extension in Chrome although Chrome removed it itself.
#[tokio::test]
async fn remove_asks_to_remove_in_chrome_only_while_the_extension_is_still_there() {
    let rig = Rig::new("chrome-addon-remove", ticket_answer).await;
    let chrome = &rig.core.shared.chrome_control;
    let data = chrome
        .config
        .user_home
        .as_ref()
        .unwrap()
        .join("chrome-data");
    let extension = data
        .join("Profile 1/Extensions")
        .join(setup::EXTENSION_ID)
        .join("0.1.3_0");
    std::fs::create_dir_all(&extension).unwrap();
    rig.core
        .chrome_control(Origin::Ui, ChromeControlRequest::Remove {})
        .await
        .unwrap();
    assert_eq!(
        rig.core.chrome_view().setup.extension_left,
        Some(true),
        "a Store-added extension stays"
    );
    std::fs::remove_dir_all(data.join("Profile 1")).unwrap();
    assert_eq!(
        rig.core.chrome_view().setup.extension_left,
        None,
        "Chrome removed it"
    );
    rig.core.shutdown().await;
}

#[cfg(unix)]
#[tokio::test]
async fn connect_restores_a_registration_after_the_browser_profile_was_reset() {
    let rig = Rig::new("connect-profile-reset", |_, _| json!({})).await;
    let chrome = &rig.core.shared.chrome_control;
    let dir = chrome.dir().unwrap();
    setup::install(&chrome.config, dir, &rig.core.shared.instructions.own_home).unwrap();
    let receipt: Value =
        serde_json::from_slice(&std::fs::read(dir.join("setup.json")).unwrap()).unwrap();
    let path = receipt["files"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .map(std::path::PathBuf::from)
        .find(|p| {
            p.file_name()
                .is_some_and(|n| n == "com.vibework.chrome_control.json")
        })
        .unwrap();
    let expected = std::fs::read(&path).unwrap();
    for damage in ["file", "folder", "contents", "start"] {
        if damage == "folder" {
            std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
        } else if damage == "contents" {
            let mut changed: Value = serde_json::from_slice(&expected).unwrap();
            changed["allowed_origins"] = json!([]);
            std::fs::write(&path, changed.to_string()).unwrap();
        } else {
            std::fs::remove_file(&path).unwrap();
        }
        if damage == "start" {
            start_owner(&rig.core.shared).await;
        } else {
            rig.connect(Origin::Ui).await;
        }
        assert_eq!(std::fs::read(&path).unwrap(), expected, "{damage}");
    }
    #[cfg(feature = "your-edge")]
    {
        let data = setup::user_data(Some(dir), "Microsoft Edge").remove(0);
        let manifest = data
            .join("NativeMessagingHosts")
            .join(format!("{}.json", setup::HOST_NAME));
        std::fs::remove_dir_all(manifest.parent().unwrap()).unwrap();
        rig.core
            .chrome_control(Origin::Ui, ChromeControlRequest::OpenEdgeStore {})
            .await
            .unwrap();
        assert_eq!(std::fs::read(manifest).unwrap(), expected);
    }
    rig.core.shutdown().await;
}

#[cfg(unix)]
#[tokio::test]
async fn an_unrepairable_host_has_the_same_recovery_action_for_settings_and_the_agent() {
    let rig = Rig::new("host-repair-state", |_, _| json!({})).await;
    let chrome = &rig.core.shared.chrome_control;
    let state = chrome.dir().unwrap();
    setup::install(
        &chrome.config,
        state,
        &rig.core.shared.instructions.own_home,
    )
    .unwrap();
    let data = setup::user_data(Some(state), "Google Chrome").remove(0);
    let extension = data
        .join("Default/Extensions")
        .join(&chrome.config.extension_id)
        .join("0.1.6");
    std::fs::create_dir_all(&extension).unwrap();
    std::fs::write(extension.join("manifest.json"), br#"{"version":"0.1.6"}"#).unwrap();
    let manifest = data
        .join("NativeMessagingHosts")
        .join(format!("{}.json", setup::HOST_NAME));
    std::fs::remove_file(&manifest).unwrap();
    let foreign = data.join("foreign.json");
    std::fs::write(&foreign, b"keep").unwrap();
    std::os::unix::fs::symlink(&foreign, &manifest).unwrap();
    let view = rig.core.chrome_view();
    let addon = view.setup.addon.unwrap();
    assert_eq!(addon.state, AddonState::HelperSetupRequired);
    assert_eq!(addon.action, Some(AddonAction::SetupAgain));
    assert_eq!(addon.label, "Helper setup required");
    assert!(
        addon
            .instruction
            .contains("registration is missing or changed")
    );
    assert!(addon.instruction.contains("Set up again"));
    assert!(addon.step.is_none());
    assert_eq!(commands::addon(&rig.core.shared), addon);
    assert!(
        rig.core
            .chrome_control(Origin::Ui, ChromeControlRequest::Connect {})
            .await
            .is_err()
    );
    assert_eq!(std::fs::read(&foreign).unwrap(), b"keep");
    std::fs::remove_file(manifest).unwrap();
    rig.connect(Origin::Ui).await;
    assert_ne!(
        commands::addon(&rig.core.shared).state,
        AddonState::HelperSetupRequired
    );
    rig.core.shutdown().await;
}
