//! Safari pairing: extension connection counts, the one-time Accessibility prompt, notice reachability, waking an idle extension and a silent paired Safari.
use super::*;
use serde_json::json;

#[tokio::test]
async fn safari_pairing_states_count_extension_connections_for_the_pairing_notice() {
    // `safari_package::open` sends the pairing notice to a just-launched Safari only after this count moves.
    let core = crate::offline_core::start_with("safari-seen", |config, dir| {
        config.chrome_control.package = Some(dir.join("package"));
    })
    .await;
    let shared = &core.shared;
    let seen = || {
        shared
            .chrome_control
            .owner
            .safari_seen
            .load(std::sync::atomic::Ordering::SeqCst)
    };
    let pairing = |profile: &str, state: &str| {
        owner::observe(
            &std::sync::Arc::downgrade(shared),
            &json!({"method":"notifications/owner/pairing", "params":{
                "profileId":profile, "state":state, "fingerprint":"a".repeat(64)}}),
        )
        .unwrap();
    };
    pairing("chrome:p", "paired");
    assert_eq!(seen(), 0);
    pairing("safari:p", "paired");
    assert_eq!(seen(), 1);
    pairing("safari:p", "waiting");
    assert_eq!(seen(), 2);
    pairing("safari:p", "unknown");
    assert_eq!(seen(), 2);
    core.shutdown().await;
}

// PREDICT (before the fix): Safari pairing completion never asks macOS for the Accessibility prompt (the counter
// stays 0); only a refused agent action prompted. After the fix: the change from `waiting` or `mismatch` to
// `paired` (the user paired in this run) asks once; a `paired` report from an already paired Safari reconnecting
// at app start, a repeated `paired`, other browsers and plain waiting states never ask.
#[tokio::test]
async fn safari_pairing_completion_asks_for_accessibility_once_and_reconnects_never_do() {
    let core = crate::offline_core::start_with("safari-ax-prompt", |config, dir| {
        config.chrome_control.package = Some(dir.join("package"));
    })
    .await;
    let shared = &core.shared;
    let asked = || {
        shared
            .chrome_control
            .owner
            .safari_accessibility_prompts
            .load(std::sync::atomic::Ordering::SeqCst)
    };
    let pairing = |profile: &str, state: &str| {
        owner::observe(
            &std::sync::Arc::downgrade(shared),
            &json!({"method":"notifications/owner/pairing", "params":{
                "profileId":profile, "state":state, "fingerprint":"a".repeat(64)}}),
        )
        .unwrap();
    };
    // An already paired Safari connecting after app start.
    pairing("safari:p", "paired");
    pairing("safari:p", "paired");
    assert_eq!(
        asked(),
        0,
        "a reconnect of an existing pairing never prompts"
    );
    pairing("chrome:p", "waiting");
    pairing("chrome:p", "paired");
    assert_eq!(asked(), 0, "other browsers never prompt");
    // The user pairs Safari in this run.
    pairing("safari:p", "waiting");
    assert_eq!(asked(), 0, "waiting for pairing does not prompt yet");
    pairing("safari:p", "paired");
    assert_eq!(asked(), 1, "pairing completion prompts once");
    pairing("safari:p", "paired");
    assert_eq!(asked(), 1, "a later paired report does not prompt again");
    // Removal clears the pairing; pairing again may prompt once more.
    *shared.chrome_control.owner.safari_pairing.lock().unwrap() = (String::new(), None);
    pairing("safari:p", "paired");
    assert_eq!(asked(), 1);
    pairing("safari:p", "mismatch");
    pairing("safari:p", "paired");
    assert_eq!(
        asked(),
        2,
        "replacing another owner's pairing is a new pairing"
    );
    core.shutdown().await;
}

// PREDICT: without the window check an idle extension counts as reachable, so "Open pairing page" with Safari already
// running reports success although Safari, not running the extension's page, drops the notice. With the check the
// notice is reachable only while Safari is launched for it or right after the extension reported; otherwise the
// person is pointed to the extension's toolbar button.
#[test]
fn the_pairing_notice_is_reachable_only_while_the_extension_page_runs() {
    use super::safari::notice_reachable;
    let now = std::time::Instant::now();
    let ago = |seconds: u64| now.checked_sub(std::time::Duration::from_secs(seconds));
    assert!(
        notice_reachable(true, None, now),
        "a launching Safari waits for the extension"
    );
    assert!(
        notice_reachable(false, ago(3), now),
        "a report 3 s ago: the page is running"
    );
    assert!(
        !notice_reachable(false, None, now),
        "no report in this app run"
    );
    assert!(
        !notice_reachable(false, ago(25), now),
        "a report older than the window"
    );
    assert!(!notice_reachable(false, ago(140), now), "idle for minutes");
}

#[tokio::test]
async fn a_safari_pairing_state_records_when_the_extension_page_last_reported() {
    let core = crate::offline_core::start_with("safari-seen-at", |config, dir| {
        config.chrome_control.package = Some(dir.join("package"));
    })
    .await;
    let shared = &core.shared;
    let at = || *shared.chrome_control.owner.safari_seen_at.lock().unwrap();
    assert_eq!(at(), None);
    let before = std::time::Instant::now();
    owner::observe(
        &std::sync::Arc::downgrade(shared),
        &json!({"method":"notifications/owner/pairing", "params":{
            "profileId":"safari:p", "state":"waiting", "fingerprint":"a".repeat(64)}}),
    )
    .unwrap();
    assert!(at().is_some_and(|at| at >= before), "{:?}", at());
    assert!(super::safari::notice_reachable(
        false,
        at(),
        std::time::Instant::now()
    ));
    core.shutdown().await;
}

#[test]
fn the_toolbar_hint_names_the_extension_button_and_the_pair_action() {
    let hint = super::safari_package::toolbar_hint();
    assert!(
        hint.contains("Your Safari") && hint.contains("Pair Vibe Work"),
        "{hint}"
    );
    assert!(!hint.contains('{'), "{hint}");
}

// Open pairing page answers an unreachable notice with guidance, not a failure.
// PREDICT (before the fix): `pairing_hint` and the result's `hint` field do not exist, so this does not compile;
// the guidance was the command's error, which the window draws as a red problem line and which also counts
// as a refusal of the command.
#[tokio::test]
async fn open_pairing_page_returns_the_toolbar_hint_as_guidance_only_when_the_notice_cannot_arrive()
{
    use super::safari_package::{pairing_hint, toolbar_hint};
    assert_eq!(pairing_hint(true), None, "the notice can arrive: no hint");
    assert_eq!(pairing_hint(false), Some(toolbar_hint()));
    let core = crate::offline_core::start_with("safari-hint", |_, _| {}).await;
    let shared = &core.shared;
    let with_hint = serde_json::to_value(crate::CommandResult::ChromeControl {
        view: super::commands::view(shared),
        offer: None,
        hint: pairing_hint(false),
    })
    .unwrap();
    assert_eq!(with_hint["type"], "chromeControl");
    assert_eq!(with_hint["hint"], json!(toolbar_hint()));
    let without = serde_json::to_value(crate::CommandResult::ChromeControl {
        view: super::commands::view(shared),
        offer: None,
        hint: None,
    })
    .unwrap();
    assert!(without.get("hint").is_none(), "{without}");
    core.shutdown().await;
}

// Open pairing page wakes an idle extension before it sends the notice.
// PREDICT (before the fix): `wake_first` and `wake_page` do not exist, so this does not compile. Natively, with Safari
// running and the extension page idle, Open pairing page only brought Safari forward: the notice was dropped and the
// person saw no pairing page. Loading any page in Safari runs the extension's page again (it listens to tab updates).
#[test]
fn an_idle_extension_is_woken_by_a_local_page_before_the_pairing_notice() {
    use super::safari::{wake_first, wake_page};
    let now = std::time::Instant::now();
    let ago = |seconds| now.checked_sub(std::time::Duration::from_secs(seconds));
    assert!(
        !wake_first(true, None, now),
        "a launch runs the page by itself"
    );
    assert!(
        !wake_first(false, ago(3), now),
        "the page reported just now"
    );
    assert!(
        wake_first(false, None, now),
        "no report since this app started"
    );
    assert!(wake_first(false, ago(25), now), "the page has been idle");
    let dir = tempfile::tempdir().unwrap();
    let page = wake_page(&dir.path().join("ChromeControl")).unwrap();
    assert!(page.starts_with(dir.path()), "{page:?}");
    assert_eq!(page.extension().and_then(|e| e.to_str()), Some("html"));
    let text = std::fs::read_to_string(&page).unwrap();
    assert!(text.contains("pairing page"), "{text}");
    assert!(
        !text.to_lowercase().contains("<script"),
        "static text only: {text}"
    );
    assert!(text.contains("default-src 'none'"), "{text}");
    // Written again on each use: a changed file never survives into the next wake.
    std::fs::write(&page, "changed").unwrap();
    assert_eq!(wake_page(&dir.path().join("ChromeControl")).unwrap(), page);
    assert_eq!(std::fs::read_to_string(&page).unwrap(), text);
}

// A paired Safari that stops answering (for example while a page's "Leave page?" sheet holds it) is not unpaired.
// PREDICT (before the fix): `diagnose` has no input for the extension's last pairing report, so this does not compile;
// natively, with no connected Safari profile the view was `NotPaired` and told the agent to have the user pair again,
// although the extension's last report was `paired`.
#[test]
fn a_paired_safari_that_goes_silent_is_shown_as_not_connected_not_unpaired() {
    let silent = safari::diagnose(true, true, None, true);
    assert_eq!(silent.state, AddonState::NotConnected);
    assert!(
        silent.instruction.contains("sheet"),
        "{}",
        silent.instruction
    );
    assert!(
        !silent.instruction.contains("Pair") && !silent.instruction.contains("pairing page"),
        "no advice to pair again: {}",
        silent.instruction
    );
    // An extension that waits for pairing, or one never heard from, is still shown as not paired.
    assert_eq!(
        safari::diagnose(true, true, None, false).state,
        AddonState::NotPaired
    );
    // A connected or switched-off Safari is unchanged.
    let version = Some(crate::version().to_owned());
    assert_eq!(
        safari::diagnose(true, true, version, true).state,
        AddonState::Connected
    );
    assert_eq!(
        safari::diagnose(true, false, None, true).state,
        AddonState::SwitchedOff
    );
}
