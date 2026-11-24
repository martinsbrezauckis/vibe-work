//! Chrome tool availability from the same state as the person's card. No setup actions run here.
//! The gateway and permission hook use these receipts; diagnoses keep the next step.
use super::{AddonState, AddonView, agent_browser::Browser};
use crate::core::Shared;
use serde_json::{Value, json};
use std::fmt::Write;

pub(super) fn reason(view: &AddonView, browser: Browser) -> String {
    let mut text = format!("Not run: {}", view.instruction);
    if browser == Browser::Firefox {
        return text;
    }
    if browser != Browser::Safari
        && !matches!(
            (browser, view.state),
            (
                _,
                AddonState::Unavailable
                    | AddonState::Connected
                    | AddonState::ChromeMissing
                    | AddonState::ChromeSandboxed
                    | AddonState::OtherBrowser
                    | AddonState::ChromeClosed
            )
        )
    {
        let _ = write!(
            text,
            " Offer the {} card with addon_suggest (addon: {}).",
            if browser == Browser::Edge && view.state == AddonState::NotSetUp {
                browser.words("{yourBrowser}")
            } else if view.state == AddonState::OwnerCredentialMissing {
                browser.name().to_owned()
            } else {
                browser.words("Connect {browser}")
            },
            browser.id()
        );
    }
    text
}

/// The refusal of a call without a connection when the shipped helper cannot start (Safari has its own helper).
fn helper_refusal(shared: &Shared, browser: Browser) -> Option<Value> {
    let problem = shared.chrome_control.config.helper_start_problem.as_ref()?;
    if browser == Browser::Safari {
        return None;
    }
    let answer = super::owner::own(&helper_cannot_start(browser, problem), "OWNER_REQUIRED");
    Some(json!({"vwMcpResult": answer.into_value()}))
}

/// The refusal when the shipped helper cannot start on this computer (`problem`: why): which part, and the repair.
pub(super) fn helper_cannot_start(browser: Browser, problem: &str) -> String {
    browser.words(&format!(
        "Not run: {{yourBrowser}} helper (chrome-control.exe) cannot start on this computer ({problem}). Vibe Work is \
         not installed completely; running the Vibe Work installer again repairs it."
    ))
}

/// Whether the helper reports a connected profile now: the owner's record of the helper's latest status answer
/// (polled every two seconds and on every call's own status read), not anything read from files.
pub(super) fn live(shared: &Shared, browser: Browser) -> bool {
    shared.chrome_control.owner.is_running() && browser.paired(shared)
}

/// The card's refusal (its diagnosis, the person's step and the `addon_suggest` hint) when Chrome cannot take a
/// call; `None` when the gateway's own checks decide. `live`: the helper reports a connected profile ([`live`], or
/// this call's own status read).
///
/// Invariants:
/// - Only the person's switch and an extension older than this app needs refuse a live connection: every other
///   non-Connected state means nothing is connected, or comes from files that can lag a live connection, so a
///   working Chrome is never refused for the card's lag.
/// - This never admits a call: the gateway's mode, lease, support and network checks run either way.
pub(super) fn refusal(shared: &Shared, browser: Browser, live: bool) -> Option<Value> {
    // Builds without Chrome keep their existing feature gate in the tool dispatcher.
    if !shared.chrome_control.config.available() {
        return None;
    }
    refusal_for(shared, browser, live, &addon(shared, browser))
}

/// [`refusal`] for a card state the caller already read.
fn refusal_for(shared: &Shared, browser: Browser, live: bool, view: &AddonView) -> Option<Value> {
    let blocks = match view.state {
        AddonState::Connected => false,
        // A closed owned Firefox is ready for session-open, not a person-only setup step.
        AddonState::ChromeClosed if browser == Browser::Firefox => false,
        // The owned runtime checks Firefox's version on every launch. A saved outdated diagnosis
        // must not block a new startup check after the user updates Firefox.
        AddonState::UpdateNeeded if browser == Browser::Firefox => !live,
        AddonState::SwitchedOff | AddonState::UpdateNeeded => true,
        _ => !live,
    };
    if !blocks {
        return None;
    }
    if let Some(refusal) = helper_refusal(shared, browser) {
        return Some(refusal);
    }
    if view.state == AddonState::Connecting && browser.reconnectable(shared) {
        return Some(super::reconnect::retry(browser));
    }
    let mut answer = super::owner::own(&reason(view, browser), "OWNER_REQUIRED").into_value();
    answer["structuredContent"]["addon"] = json!({
        "state": view.label, "stateCode": view.state, "instruction": view.instruction,
        "extensionVersion":view.extension_version,
    });
    Some(json!({"vwMcpResult":answer}))
}

/// The card's refusal before a call is sent: [`refusal`], except while Chrome is connecting. The pairing was just
/// confirmed and the extension's connection is forming, so the helper's own answer decides; a failure it answers
/// gets the card's words afterwards (`gateway::call_one`). Seen in the Chrome e2e: right after Use my Chrome was
/// turned off and on, the card said Connecting and refused the chat's calls; with the helper deciding, they ran.
pub(super) fn gate(shared: &Shared, browser: Browser, live: bool) -> Option<Value> {
    if !shared.chrome_control.config.available() {
        return None;
    }
    // A helper that cannot start never connects: say so before the connecting state lets the call wait for it.
    if !live && let Some(refusal) = helper_refusal(shared, browser) {
        return Some(refusal);
    }
    let view = addon(shared, browser);
    if view.state == AddonState::Connecting {
        return None;
    }
    refusal_for(shared, browser, live, &view)
}

/// The browser's card state. Each read rebuilds the Browser settings view (setup receipt, native-host registration
/// check, helper check), so a check reads it once; tests count the reads.
fn addon(shared: &Shared, browser: Browser) -> AddonView {
    #[cfg(test)]
    tests::VIEWS.with(|reads| reads.set(reads.get() + 1));
    browser.addon(shared)
}

/// The connection's current diagnosis after a read, without overwriting an uncertain action receipt.
pub(super) fn after_call(
    shared: &Shared,
    browser: Browser,
    tool: &str,
    request: bool,
    answer: &super::owner::Answer,
) -> Option<Value> {
    if tool == "chrome_status"
        && let Some(refusal) = super::availability::refusal(
            shared,
            browser,
            super::availability::live(shared, browser),
        )
    {
        return Some(refusal);
    }
    // Before its first session Firefox needs no running helper: the session open starts it. A status
    // query is not a failed browser action and must not tell the user to restart a healthy app.
    if tool == "chrome_status"
        && browser == Browser::Firefox
        && !shared.chrome_control.owner.is_running()
        && answer.value()["structuredContent"]["error"]["code"] == "OWNER_REQUIRED"
    {
        let view = addon(shared, browser);
        if view.state == AddonState::ChromeClosed {
            return Some(json!({"vwMcpResult":{
                "isError":false,
                "content":[{"type":"text","text":view.instruction}],
                "structuredContent":{"firefoxWindow":"Window closed","profiles":[],"instruction":view.instruction},
            }}));
        }
    }
    // If the connection changed during the call, report the card's current diagnosis too; never for an unknown
    // outcome (the connection ended mid-call): the action may have run, so it must not read as not run.
    let error = &answer.value()["structuredContent"]["error"];
    let reconnecting = browser.reconnectable(shared)
        && browser.addon(shared).state == super::AddonState::Connecting;
    let not_ready = request && error["code"] == "SESSION_NOT_READY";
    if request && !not_ready {
        shared.chrome_control.owner.not_ready.opened(browser);
    }
    if answer.value()["structuredContent"]["status"] != "unknown"
        && ((reconnecting && error["code"] == "PROFILE_NOT_CONNECTED")
            || (not_ready && browser.reconnectable(shared)))
    {
        // A connected browser that refuses starts again is not reconnecting: say so, and ask the person.
        if not_ready && !reconnecting && shared.chrome_control.owner.not_ready.repeated(browser) {
            return Some(super::reconnect::unconfirmed(browser));
        }
        return Some(super::reconnect::pending(browser));
    }
    if !error.is_null()
        && !reconnecting
        && error["code"] != "TRANSPORT_LOST"
        && answer.value()["structuredContent"]["status"] != "unknown"
        && let Some(refusal) = super::availability::refusal(
            shared,
            browser,
            super::availability::live(shared, browser),
        )
    {
        return Some(refusal);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chrome_control::{gateway, owner, test_support};

    thread_local! {
        /// Card-state reads by this module on this test's thread (a current-thread runtime keeps the call here).
        pub(super) static VIEWS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }

    // PREDICT (before the gate change): one page.read reads the card state three times: once in the gateway's
    // local check and twice in the pre-send gate (the Connecting test, then `refusal` again). Each read rebuilds the
    // Browser settings view (receipt, registration check, helper check); the gate needs one, so a read costs two.
    #[tokio::test]
    async fn a_page_read_reads_the_card_state_once_per_check() {
        let core = crate::chrome_control::output_tests::core("availability-view-reads").await;
        test_support::answering(&core.shared, |tool, args| {
            if tool == "chrome_status" {
                crate::chrome_control::output_tests::status(args)
            } else {
                crate::chrome_control::output_tests::mcp(
                    json!({"status":"completed","data":{"textPreview":"quay"}}),
                )
            }
        });
        VIEWS.with(|reads| reads.set(0));
        let reply = gateway::call(
            &core.shared,
            "s",
            "chrome_execute",
            json!({"sessionId":"t","action":"page.read","args":{}}),
        )
        .await
        .unwrap();
        assert!(
            reply["vwMcpResult"]["structuredContent"]["error"].is_null(),
            "{reply}"
        );
        assert_eq!(
            VIEWS.with(std::cell::Cell::get),
            2,
            "the local check and the gate"
        );
        core.shutdown().await;
    }

    /// Each test names its own core: one folder per name and process, so parallel tests never share files.
    async fn core_with_min_version(name: &str) -> crate::Core {
        crate::offline_core::start_with(name, |config, dir| {
            config.chrome_control.package = Some(dir.join("package"));
            let root = dir.join("install");
            vw_vibe_install::extension_store::save_record(
                &root,
                Some(vw_vibe_install::support::ChromeExtension {
                    store: Some(vw_vibe_install::ExtensionStore::Available),
                    min_version: Some("0.1.6".into()),
                }),
            )
            .unwrap();
            config.vibe_install = Some(vw_vibe_install::VibeInstaller::new(
                vw_vibe_install::InstallerConfig {
                    root,
                    sources: vw_vibe_install::Sources::pinned(&vw_vibe_install::this_target()),
                    parent_env: Vec::new(),
                    extra_env: Vec::new(),
                    discovery: vw_supervisor::Discovery {
                        explicit: None,
                        path_var: None,
                        home: None,
                    },
                    limits: vw_vibe_install::Limits::default(),
                },
            ));
        })
        .await
    }

    #[test]
    fn a_closed_browser_explains_the_app_launcher_and_the_agents_next_step() {
        for browser in [Browser::Chrome, Browser::Edge] {
            let view = crate::chrome_control::addon::view_for(
                AddonState::ChromeClosed,
                crate::chrome_control::setup::ABSENT,
                browser,
            );
            let text = reason(&view, browser);
            assert!(text.contains("is not open") && text.contains("Call chrome_session"));
            assert!(
                text.contains("Vibe Work opens it")
                    && text.contains("Do not start the browser with process or shell tools")
            );
            assert!(!text.contains("addon_suggest"));
        }
    }

    #[tokio::test]
    async fn tools_report_the_cards_diagnosis_and_instruction() {
        let core = core_with_min_version("chrome-unavailable-tools").await;
        let shared = &core.shared;
        let _ = shared
            .store
            .load(
                json!({"session":{"id":"s"},"history":[],"turns":[]}),
                vw_session::LoadKind::New,
            )
            .unwrap();
        shared.access.observe_request(
            "session/agent/update",
            "s",
            &json!({"agentName":"auto-approve"}),
        );
        let calls = test_support::answering(shared, |_, _| {
            panic!("unavailable tools must not reach Chrome")
        });
        let chrome = &shared.chrome_control;
        chrome
            .owner
            .tools
            .lock()
            .unwrap()
            .push(json!({"name":"chrome_status"}));
        let dir = chrome.dir().unwrap();
        let data = chrome
            .config
            .user_home
            .as_ref()
            .unwrap()
            .join("chrome-data");
        let profile = data.join("Profile 1");
        let extension = profile.join("Extensions").join(&chrome.config.extension_id);
        std::fs::write(
            dir.join("setup.json"),
            json!({"files":[],"browsers":["Google Chrome"],"userData":[data]}).to_string(),
        )
        .unwrap();
        chrome
            .set_browser_enabled(super::super::agent_browser::Browser::Edge, false)
            .unwrap();
        for state in [
            AddonState::SwitchedOff,
            AddonState::ExtensionNotInstalled,
            AddonState::TurnedOffInChrome,
            AddonState::UpdateNeeded,
            AddonState::NotPaired,
        ] {
            chrome
                .set_enabled(state != AddonState::SwitchedOff)
                .unwrap();
            chrome.owner.pairing.lock().unwrap().0 =
                crate::chrome_control::PairingState::Disconnected;
            if state == AddonState::TurnedOffInChrome {
                std::fs::create_dir_all(&extension).unwrap();
                std::fs::write(profile.join("Preferences"), json!({"extensions":{"settings":{&chrome.config.extension_id:{"disable_reasons":[1]}}}}).to_string()).unwrap();
            }
            if state == AddonState::UpdateNeeded {
                std::fs::remove_file(profile.join("Preferences")).unwrap();
                std::fs::create_dir_all(extension.join("0.1.3_0")).unwrap();
                // An old connected extension is still too old for this app: the live profile reports its version.
                chrome.owner.pairing.lock().unwrap().0 =
                    crate::chrome_control::PairingState::Paired;
                chrome.owner.connected_profiles.observe(&json!({"profiles":[
                    {"profileId":"p","connected":true,"extensionVersion":"0.1.3"}]}));
            }
            if state == AddonState::NotPaired {
                owner::observe(&std::sync::Arc::downgrade(shared), &json!({"method":"notifications/owner/pairing", "params":{"profileId":"chrome:p","state":"waiting","fingerprint":"fixture"}})).unwrap();
            }
            let card = crate::chrome_control::commands::addon(shared);
            assert_eq!(card.state, state);
            for (tool, args) in [
                ("chrome_status", json!({})),
                ("chrome_session", json!({"action":"open","profileId":"p"})),
                (
                    "chrome_execute",
                    json!({"sessionId":"old","action":"page.read","args":{}}),
                ),
            ] {
                let reply = gateway::call(shared, "s", tool, args).await.unwrap();
                let text = reply["vwMcpResult"]["content"][0]["text"].as_str().unwrap();
                assert!(
                    text.contains(&card.instruction) && text.contains("addon_suggest"),
                    "{state:?} {tool}: {text}"
                );
                assert_eq!(
                    reply["vwMcpResult"]["structuredContent"]["addon"]["stateCode"],
                    json!(state)
                );
            }
        }
        assert!(calls.lock().unwrap().is_empty());
        core.shutdown().await;
    }
    /// An extension older than this app needs keeps refusing the new actions: through the live feature gate while the
    /// card does not know the connected profile's version yet, and with the card's words once it reports it.
    #[tokio::test]
    async fn an_outdated_extension_keeps_refusing_the_new_actions() {
        let core = core_with_min_version("chrome-outdated-new-actions").await;
        let shared = &core.shared;
        let _ = shared
            .store
            .load(
                json!({"session":{"id":"s"},"history":[],"turns":[]}),
                vw_session::LoadKind::New,
            )
            .unwrap();
        shared.access.observe_request(
            "session/agent/update",
            "s",
            &json!({"agentName":"auto-approve"}),
        );
        let chrome = &shared.chrome_control;
        chrome.tasks.lock().unwrap().insert("t".into(), "s".into());
        let old = json!({"profileId":"p","connected":true,"extensionVersion":"0.1.3",
            "features":["page.read"],"extensionCapabilities":["page.read"]});
        let status = json!({"structuredContent":{"sessions":[{"sessionId":"t","profileId":"p"}],"profiles":[old]}});
        let seen = test_support::answering(shared, move |tool, _| {
            if tool == "chrome_status" {
                status.clone()
            } else {
                json!({"structuredContent":{"status":"completed"}})
            }
        });
        let args = json!({"sessionId":"t","action":"clipboard.read","args":{}});
        assert_eq!(
            crate::chrome_control::commands::addon(shared).state,
            AddonState::Connected
        );
        let reason = gateway::call(shared, "s", "chrome_execute", args.clone())
            .await
            .unwrap_err();
        assert!(reason.contains("does not report support"), "{reason}");
        chrome
            .owner
            .connected_profiles
            .observe(&json!({"profiles":[old]}));
        assert_eq!(
            crate::chrome_control::commands::addon(shared).state,
            AddonState::UpdateNeeded
        );
        let reply = gateway::call(shared, "s", "chrome_execute", args)
            .await
            .unwrap();
        assert_eq!(
            reply["vwMcpResult"]["structuredContent"]["addon"]["stateCode"],
            json!(AddonState::UpdateNeeded),
            "{reply}"
        );
        assert!(
            seen.lock()
                .unwrap()
                .iter()
                .all(|(name, _)| name == "chrome_status")
        );
        core.shutdown().await;
    }
}

// PREDICT (helper that cannot start): Windows could not load chrome-control.exe, so the extension never connected and
// an agent's browser call got the card's words (not set up / not connected), naming neither the helper nor the repair.
#[cfg(test)]
mod helper_start_tests {
    use crate::chrome_control::gateway;
    use serde_json::json;

    const PROBLEM: &str = "Windows could not load it: error 0xC0000135, a DLL it needs is missing";

    async fn core(label: &str, problem: Option<&str>) -> crate::core::Core {
        let problem = problem.map(str::to_owned);
        let core = crate::offline_core::start_with(label, move |config, dir| {
            config.chrome_control.package = Some(dir.join("package"));
            config.chrome_control.helper_start_problem = problem;
        })
        .await;
        let _ = core
            .shared
            .store
            .load(
                json!({"session":{"id":"s"},"history":[],"turns":[]}),
                vw_session::LoadKind::New,
            )
            .unwrap();
        core.shared.access.observe_request(
            "session/agent/update",
            "s",
            &json!({"agentName":"auto-approve"}),
        );
        core.shared
            .chrome_control
            .tasks
            .lock()
            .unwrap()
            .insert("t".into(), "s".into());
        core
    }

    async fn answers(core: &crate::core::Core) -> Vec<String> {
        let mut texts = Vec::new();
        for (tool, args) in [
            ("chrome_status", json!({"sessionId":"t"})),
            (
                "chrome_execute",
                json!({"sessionId":"t","action":"page.read","args":{}}),
            ),
        ] {
            texts.push(format!(
                "{:?}",
                gateway::call(&core.shared, "s", tool, args).await
            ));
        }
        texts
    }

    #[tokio::test]
    async fn an_agent_call_names_the_helper_that_cannot_start_and_the_repair() {
        let expected = format!(
            "Not run: {} helper (chrome-control.exe) cannot start on this computer ({PROBLEM}). Vibe Work is \
             not installed completely; running the Vibe Work installer again repairs it.",
            vw_vocabulary::texts::CHROME_NAME
        );
        let core = core("availability-helper-cannot-start", Some(PROBLEM)).await;
        for text in answers(&core).await {
            assert!(text.contains(&expected), "{text}");
        }
        // Falsifier: the same core without the problem answers with the card's words.
        let core = core_without().await;
        for text in answers(&core).await {
            assert!(!text.contains("chrome-control.exe"), "{text}");
        }
    }

    async fn core_without() -> crate::core::Core {
        core("availability-helper-starts", None).await
    }

    #[test]
    fn each_browser_names_its_own_card() {
        use crate::chrome_control::agent_browser::Browser;
        assert!(
            super::helper_cannot_start(Browser::Edge, "x").starts_with(&format!(
                "Not run: {} helper (chrome-control.exe)",
                vw_vocabulary::texts::EDGE_NAME
            ))
        );
    }
}
