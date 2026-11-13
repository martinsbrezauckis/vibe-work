//! Paired profiles carry the browser's own profile name, and the signed-in account only for the screen; agent text
//! adds the account's domain.
//!
//! PREDICT (written before the change): every test here fails on the base. `profile_names` does not exist, so the
//! `Local State` parsing tests do not compile; the inventory names an unproven or unnamed identity
//! `Profile <8 hex>` (a hash of its ID) and two same-named profiles `Work · <8 hex>`, never `Chrome profile 1` or the
//! name Chrome shows in its own profile picker; and `BrowserProfile` has no account field, so the email Chrome shows
//! under the name cannot reach Settings. After the change: names come from the proven folder's `Local State` entry,
//! fall back to `Chrome profile N` / `Edge profile N` in pairing order, old saved names never cover Chrome's name,
//! and no agent-visible text carries the account.
//!
//! Uses temporary browser folders laid out like Chrome's, no native browser.
#![allow(
    clippy::unwrap_used,
    reason = "temporary browser folders and fixtures must be created for the test to mean anything"
)]
use super::{edge_agent_tests::fixture, profile_names, test_support};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub(super) const WORK_UUID: &str = "1b4e28ba-2fa1-41d2-883f-0016d3cca427";
pub(super) const HOME_UUID: &str = "6fa459ea-ee8a-4ca4-894e-db77e160355e";
pub(super) const WORK: &str = "chrome:1b4e28ba-2fa1-41d2-883f-0016d3cca427";
pub(super) const HOME: &str = "chrome:6fa459ea-ee8a-4ca4-894e-db77e160355e";

fn hash_name(text: &str) -> bool {
    // `Profile 1a2b3c4d` or `· 1a2b3c4d`: a name made from an identity hash.
    text.split(|c: char| !c.is_ascii_alphanumeric())
        .any(|word| {
            word.len() == 8
                && word.chars().all(|c| c.is_ascii_hexdigit())
                && word.chars().any(|c| c.is_ascii_digit())
        })
}

#[test]
fn local_state_gives_each_folder_its_name_and_signed_in_account() {
    let state = json!({"profile":{"info_cache":{
        "Profile 1":{"name":"Personal","user_name":"","gaia_name":""},
        "Profile 3":{"name":"Work","user_name":"work@example.com","gaia_name":"Alex Example"},
        "Profile 4":{"name":"","gaia_name":"Sam Example","user_name":"sam@example.com"},
        "Profile 5":{"name":"Bad\u{7}Name"},
        "Profile 6":{"avatar_icon":"chrome://theme/IDR_PROFILE_AVATAR_26"}
    }}});
    let names = profile_names::parse(state.to_string().as_bytes()).unwrap();
    let personal = &names["Profile 1"];
    assert_eq!(personal.name.as_deref(), Some("Personal"));
    assert_eq!(
        personal.account, None,
        "a profile without sign-in has no account line"
    );
    let work = &names["Profile 3"];
    assert_eq!(work.name.as_deref(), Some("Work"));
    assert_eq!(work.account.as_deref(), Some("work@example.com"));
    // An empty local name falls back to the signed-in account's own name, as Chrome's picker does.
    assert_eq!(names["Profile 4"].name.as_deref(), Some("Sam Example"));
    assert_eq!(
        names["Profile 4"].account.as_deref(),
        Some("sam@example.com")
    );
    // A policy's enterprise label is the name Chrome shows, before the local name.
    let labelled = profile_names::parse(
        json!({"profile":{"info_cache":{"Profile 2":{"name":"Person 1","enterprise_label":"Acme work"}}}})
            .to_string()
            .as_bytes(),
    )
    .unwrap();
    assert_eq!(labelled["Profile 2"].name.as_deref(), Some("Acme work"));
    // Control characters and a missing name never become a label.
    assert_eq!(names["Profile 5"].name, None);
    assert_eq!(names["Profile 6"].name, None);
    assert!(!names.contains_key("Profile 9"));
}

#[test]
fn malformed_or_foreign_local_state_names_nothing() {
    for bytes in [
        b"{not json".as_slice(),
        b"[]".as_slice(),
        b"{\"profile\":{}}".as_slice(),
        b"{\"profile\":{\"info_cache\":[]}}".as_slice(),
        b"".as_slice(),
    ] {
        assert!(
            profile_names::parse(bytes).is_none_or(|names| names.is_empty()),
            "{}",
            String::from_utf8_lossy(bytes)
        );
    }
    let long = "x".repeat(200);
    let names = profile_names::parse(
        json!({"profile":{"info_cache":{"Profile 1":{"name":long,"user_name":"a\u{0}@b"}}}})
            .to_string()
            .as_bytes(),
    )
    .unwrap();
    assert_eq!(
        names["Profile 1"].name, None,
        "an oversized name is not shown"
    );
    assert_eq!(
        names["Profile 1"].account, None,
        "a control character is not shown"
    );
}

/// One Chrome user data folder registered by Set up. Each `(folder, uuid)` gets the extension's `IndexedDB` holding
/// that identity's UUID (the per-install marker); `Local State` lists `entries`.
pub(super) fn browser_data(
    core: &crate::Core,
    folders: &[(&str, &str)],
    entries: &Value,
) -> PathBuf {
    let chrome = &core.shared.chrome_control;
    let data = chrome
        .config
        .user_home
        .as_ref()
        .unwrap()
        .join("names-browser-data");
    let id = &chrome.config.extension_id;
    let origin = format!("chrome-extension_{id}_0");
    for (folder, uuid) in folders {
        let folder = data.join(folder);
        std::fs::create_dir_all(folder.join("Extensions").join(id).join("0.1.7_0")).unwrap();
        let storage = folder.join("IndexedDB").join(&origin);
        std::fs::create_dir_all(&storage).unwrap();
        std::fs::write(
            storage.join("WCHYFF2I4HCHMUPPZAC67CSVYI6DH5O5Q6INA6VYGXVHAVFYHNPQ"),
            format!("SQLite format 3\0\u{ff}\u{14}\"${uuid}"),
        )
        .unwrap();
    }
    write_local_state(&data, entries);
    std::fs::create_dir_all(chrome.dir().unwrap()).unwrap();
    std::fs::write(
        chrome.dir().unwrap().join("setup.json"),
        json!({"files":[],"browsers":["Google Chrome"],"userData":[data]}).to_string(),
    )
    .unwrap();
    data
}

fn write_local_state(data: &Path, entries: &Value) {
    std::fs::write(
        data.join("Local State"),
        json!({"profile":{"info_cache":entries}}).to_string(),
    )
    .unwrap();
}

pub(super) fn observe(core: &crate::Core, ids: &[&str]) {
    let rows: Vec<Value> = ids
        .iter()
        .map(|id| json!({"profileId":id,"connected":true}))
        .collect();
    test_support::observe_status(&core.shared, &json!({"profiles":rows}));
}

pub(super) fn names(core: &crate::Core) -> Vec<(String, String, Option<String>)> {
    super::profiles::view(&core.shared)
        .into_iter()
        .map(|row| (row.profile_id, row.name, row.account))
        .collect()
}

#[tokio::test]
async fn a_proven_folder_shows_chromes_profile_name_and_account() {
    let core = fixture("names-proven", json!([])).await;
    browser_data(
        &core,
        &[("Profile 3", WORK_UUID), ("Profile 1", HOME_UUID)],
        &json!({
            "Profile 3":{"name":"Work","user_name":"work@example.com"},
            "Profile 1":{"name":"Personal","user_name":""}
        }),
    );
    observe(&core, &[WORK, HOME]);
    let rows = names(&core);
    assert!(
        rows.contains(&(WORK.into(), "Work".into(), Some("work@example.com".into()))),
        "{rows:?}"
    );
    assert!(
        rows.contains(&(HOME.into(), "Personal".into(), None)),
        "{rows:?}"
    );
    core.shutdown().await;
}

#[tokio::test]
async fn a_renamed_chrome_profile_is_renamed_here_and_old_saved_names_never_cover_it() {
    let core = fixture("names-renamed", json!([])).await;
    let data = browser_data(
        &core,
        &[("Profile 3", WORK_UUID)],
        &json!({"Profile 3":{"name":"Work"}}),
    );
    observe(&core, &[WORK]);
    // A name the person typed in an earlier version is only a fallback now.
    core.shared
        .chrome_control
        .profiles
        .lock()
        .unwrap()
        .records
        .get_mut(WORK)
        .unwrap()
        .name = Some("Typed earlier".into());
    assert_eq!(names(&core)[0].1, "Work");
    // `Local State` is rewritten when the person renames the profile in Chrome; the file's time or size changes.
    write_local_state(&data, &json!({"Profile 3":{"name":"Office laptop"}}));
    observe(&core, &[WORK]);
    assert_eq!(names(&core)[0].1, "Office laptop");
    core.shutdown().await;
}

#[tokio::test]
async fn an_unreadable_name_falls_back_to_the_browser_and_pairing_order() {
    let core = fixture("names-fallback", json!([])).await;
    // No folder holds either identity, so neither can be named from `Local State`.
    observe(&core, &[HOME]);
    observe(&core, &[HOME, WORK]);
    let rows = names(&core);
    let name = |id: &str| rows.iter().find(|row| row.0 == id).unwrap().1.clone();
    assert_eq!(name(HOME), "Chrome profile 1");
    assert_eq!(name(WORK), "Chrome profile 2");
    // Rows follow pairing order (PREDICT: fails before the sort; rows followed the identity order, WORK first).
    let order: Vec<_> = rows.iter().map(|row| row.1.as_str()).collect();
    assert_eq!(order, ["Chrome profile 1", "Chrome profile 2"]);
    for row in &rows {
        assert!(!hash_name(&row.1), "hash-like name {row:?}");
        assert_eq!(row.2, None);
    }
    core.shutdown().await;
}

#[tokio::test]
async fn edge_profiles_are_numbered_on_their_own() {
    let core = fixture("names-edge", json!([])).await;
    observe(&core, &[WORK, "edge:6fa459ea-ee8a-4ca4-894e-db77e160355e"]);
    let rows = names(&core);
    assert!(
        rows.iter()
            .any(|row| row.0.starts_with("edge:") && row.1 == "Edge profile 1"),
        "{rows:?}"
    );
    assert!(
        rows.iter()
            .any(|row| row.0 == WORK && row.1 == "Chrome profile 1"),
        "{rows:?}"
    );
    core.shutdown().await;
}

#[tokio::test]
async fn same_named_profiles_are_told_apart_without_a_hash() {
    let core = fixture("names-duplicate", json!([])).await;
    browser_data(
        &core,
        &[("Profile 3", WORK_UUID), ("Profile 1", HOME_UUID)],
        &json!({
            "Profile 3":{"name":"Work","user_name":"a@example.com"},
            "Profile 1":{"name":"Work","user_name":"b@example.com"}
        }),
    );
    observe(&core, &[WORK, HOME]);
    let rows = names(&core);
    // The screen shows each name with its account, so different accounts already tell the two apart.
    let mut labels: Vec<_> = rows
        .iter()
        .map(|row| (row.1.clone(), row.2.clone()))
        .collect();
    labels.sort();
    labels.dedup();
    assert_eq!(labels.len(), 2, "{rows:?}");
    for row in &rows {
        assert!(row.1.starts_with("Work"), "{rows:?}");
        assert!(!hash_name(&row.1), "hash-like name {row:?}");
    }
    core.shutdown().await;
}

#[tokio::test]
async fn agent_visible_profile_text_names_the_profile_and_domain_but_never_the_account() {
    let core = fixture("names-agent-private", json!([])).await;
    browser_data(
        &core,
        &[("Profile 3", WORK_UUID), ("Profile 1", HOME_UUID)],
        &json!({
            "Profile 3":{"name":"Work","user_name":"work@example.com"},
            "Profile 1":{"name":"Personal","user_name":"me@example.com"}
        }),
    );
    observe(&core, &[WORK, HOME]);
    let mut result = json!({"structuredContent":{},"content":[]});
    super::profile_agent::status(&core.shared, "s", &mut result);
    let text = result.to_string();
    assert!(text.contains("Work") && text.contains("Personal"), "{text}");
    assert!(
        text.contains("Work (example.com)") && text.contains("Personal (example.com)"),
        "{text}"
    );
    assert!(!text.contains('@'), "{text}");
    assert!(
        !hash_name(&text.replace(WORK_UUID, "").replace(HOME_UUID, "")),
        "{text}"
    );
    let unavailable = super::profile_agent::unavailable(&core.shared, WORK);
    assert!(
        unavailable.contains("Work (example.com)") && !unavailable.contains('@'),
        "{unavailable}"
    );
    core.shutdown().await;
}

/// PREDICT: fails before the fix. The second identity's UUID reaches its folder's storage after the first read of
/// `Local State`, which then does not change, so the second profile stays "Chrome profile 2" although its folder is
/// proven. After the fix a newly proven folder is named from the same `Local State` at once.
#[tokio::test]
async fn a_folder_proven_later_is_named_without_a_change_to_local_state() {
    let core = fixture("names-proven-later", json!([])).await;
    let data = browser_data(
        &core,
        &[("Profile 3", WORK_UUID), ("Profile 1", "not-yet-written")],
        &json!({
            "Profile 3":{"name":"Work"},
            "Profile 1":{"name":"Personal"}
        }),
    );
    observe(&core, &[WORK, HOME]);
    let name =
        |core: &crate::Core, id: &str| names(core).into_iter().find(|row| row.0 == id).unwrap().1;
    assert_eq!(name(&core, WORK), "Work");
    assert_eq!(name(&core, HOME), "Chrome profile 2");
    // The extension stores the second identity's UUID now; `Local State` is untouched.
    let id = core.shared.chrome_control.config.extension_id.clone();
    std::fs::write(
        data.join("Profile 1/IndexedDB")
            .join(format!("chrome-extension_{id}_0"))
            .join("WCHYFF2I4HCHMUPPZAC67CSVYI6DH5O5Q6INA6VYGXVHAVFYHNPQ"),
        format!("SQLite format 3\0\u{ff}\u{14}\"${HOME_UUID}"),
    )
    .unwrap();
    // The next search for unproven identities (normally a minute later) finds it.
    core.shared.chrome_control.profiles.lock().unwrap().scanned = None;
    let mut events = core.shared.events.subscribe();
    // The search runs on every helper message; not every message publishes the browser view itself.
    super::profile_presence::reconcile(&core.shared);
    assert_eq!(name(&core, HOME), "Personal");
    // The open window learns the new name at once (PREDICT: fails before the publish; Settings kept showing
    // "Chrome profile 2" until some other change was published).
    let mut published = false;
    while let Ok(event) = events.try_recv() {
        if let crate::CoreEvent::ChromeControl { view } = event {
            published = view
                .paired_profiles
                .as_ref()
                .is_some_and(|rows| rows.iter().any(|row| row.name == "Personal"));
        }
    }
    assert!(published, "the window must receive the browser's new name");
    core.shutdown().await;
}

/// A browser writes a newly paired profile's extension storage a moment after pairing, so the first search can miss
/// the folder. PREDICT: fails before the fix — the next search came only a minute later, and Settings showed
/// "Chrome profile 2" for that minute although the browser already had the profile's name.
#[tokio::test]
async fn a_newly_paired_folder_is_searched_again_within_seconds() {
    let core = fixture("names-retry", json!([])).await;
    let data = browser_data(
        &core,
        &[("Profile 3", WORK_UUID), ("Profile 1", "not-yet-written")],
        &json!({
            "Profile 3":{"name":"Work"},
            "Profile 1":{"name":"Personal"}
        }),
    );
    // The first profile is paired and named; the second is paired later and its first search misses.
    observe(&core, &[WORK]);
    observe(&core, &[WORK, HOME]);
    super::profile_presence::reconcile(&core.shared);
    let name =
        |core: &crate::Core, id: &str| names(core).into_iter().find(|row| row.0 == id).unwrap().1;
    assert_eq!(name(&core, WORK), "Work");
    assert_eq!(name(&core, HOME), "Chrome profile 2");
    let id = core.shared.chrome_control.config.extension_id.clone();
    std::fs::write(
        data.join("Profile 1/IndexedDB")
            .join(format!("chrome-extension_{id}_0"))
            .join("WCHYFF2I4HCHMUPPZAC67CSVYI6DH5O5Q6INA6VYGXVHAVFYHNPQ"),
        format!("SQLite format 3\0\u{ff}\u{14}\"${HOME_UUID}"),
    )
    .unwrap();
    // A few seconds pass; the set of unproven identities is unchanged.
    let aged = |core: &crate::Core, seconds: u64| {
        if let Some(scan) = core
            .shared
            .chrome_control
            .profiles
            .lock()
            .unwrap()
            .scanned
            .as_mut()
        {
            scan.0 = scan
                .0
                .checked_sub(std::time::Duration::from_secs(seconds))
                .unwrap();
        }
    };
    aged(&core, 3);
    super::profile_presence::reconcile(&core.shared);
    assert_eq!(name(&core, HOME), "Personal");
    core.shutdown().await;
}

/// An identity whose folder is never found is not searched on every helper message: the quick retries end and the
/// search falls back to once a minute.
#[tokio::test]
async fn an_unprovable_identity_is_searched_at_most_once_a_minute_after_the_retries() {
    let core = fixture("names-retry-cap", json!([])).await;
    browser_data(
        &core,
        &[("Profile 3", WORK_UUID), ("Profile 1", "never-written")],
        &json!({"Profile 3":{"name":"Work"}}),
    );
    observe(&core, &[WORK]);
    observe(&core, &[WORK, HOME]);
    // Moves the last search `seconds` into the past and returns when it now appears to have run.
    let age = |core: &crate::Core, seconds: u64| {
        let mut saved = core.shared.chrome_control.profiles.lock().unwrap();
        let scan = saved.scanned.as_mut().unwrap();
        scan.0 = scan
            .0
            .checked_sub(std::time::Duration::from_secs(seconds))
            .unwrap();
        scan.0
    };
    let last = |core: &crate::Core| {
        core.shared
            .chrome_control
            .profiles
            .lock()
            .unwrap()
            .scanned
            .as_ref()
            .unwrap()
            .0
    };
    // Each retry delay passes in turn: every reconcile searches again while retries remain.
    for _ in 0..10 {
        age(&core, 25);
        super::profile_presence::reconcile(&core.shared);
    }
    // Retries spent: 25 s (longer than any retry delay) is not yet a reason to search again.
    let aged = age(&core, 25);
    super::profile_presence::reconcile(&core.shared);
    assert_eq!(last(&core), aged, "no search before a minute has passed");
    // A minute after the last search it runs again.
    age(&core, 40);
    super::profile_presence::reconcile(&core.shared);
    assert!(last(&core).elapsed() < std::time::Duration::from_secs(5));
    core.shutdown().await;
}

const MANAGED: &str = "chrome:9a3c1e2f-5b6d-4e7f-8a9b-0c1d2e3f4a5b";
const TYPED: &str = "chrome:0d6c8a4e-1f2b-4c3d-9e8f-7a6b5c4d3e2f";
const UNSIGNED: &str = "chrome:4b7e9c1d-2a3f-4e5d-8c7b-6a5f4e3d2c1b";

/// Five signed-in, managed, typed-email and signed-out profiles of one Chrome, all connected.
async fn domain_fixture() -> crate::Core {
    let core = fixture("names-agent-domain", json!([])).await;
    browser_data(
        &core,
        &[
            ("Profile 3", WORK_UUID),
            ("Profile 1", HOME_UUID),
            ("Profile 4", "9a3c1e2f-5b6d-4e7f-8a9b-0c1d2e3f4a5b"),
            ("Profile 5", "0d6c8a4e-1f2b-4c3d-9e8f-7a6b5c4d3e2f"),
            ("Profile 6", "4b7e9c1d-2a3f-4e5d-8c7b-6a5f4e3d2c1b"),
        ],
        &json!({
            "Profile 3":{"name":"Alex","user_name":"alex@example.com","gaia_name":"Alex Example",
                "hosted_domain":"NO_HOSTED_DOMAIN","is_managed":0},
            "Profile 1":{"name":"Work","user_name":"alex.work@Company.Example","gaia_name":"Alex Example",
                "hosted_domain":"company.example","is_managed":1},
            "Profile 4":{"name":"company.example","user_name":"jane@company.example",
                "hosted_domain":"company.example","is_managed":1},
            "Profile 5":{"name":"jane@company.example","user_name":"jane@company.example",
                "hosted_domain":"company.example"},
            "Profile 6":{"name":"Your Chrome","user_name":"","gaia_name":"","hosted_domain":""}
        }),
    );
    observe(&core, &[WORK, HOME, MANAGED, TYPED, UNSIGNED]);
    core
}

/// Agent text names a profile as Chrome shows it, plus the domain of the signed-in account in brackets: "Alex
/// (company.example)". A personal account shows its mail domain ("example.com"), a managed profile Chrome named after
/// its domain shows that name once, a signed-out profile shows its name alone, and a name the person typed as an email
/// is shown as typed (it is what Chrome shows). The account itself (`user_name`) never reaches agent text. Settings and
/// the person's card keep the name and the account.
///
/// PREDICT (written before the change): fails. Agent text replaces every name that contains '@' or the account with
/// "Chrome profile N" and never adds a domain, so `profileChoices`/`selectedProfile`, the unavailable, removed and
/// "several profiles" texts, the inventory names, the receipt's `profileName` and the card's "could not be shown"
/// text say "Chrome · Alex" and "Chrome profile 4" instead of "Chrome · Alex (example.com)" and
/// "jane@company.example"; and `parse` ignores `enterprise_label`, the name Chrome shows for a profile whose policy
/// sets one.
#[tokio::test]
async fn agent_text_names_a_profile_as_chrome_shows_it_with_the_account_domain() {
    let core = domain_fixture().await;
    // The person's own view keeps the browser's name and the account.
    let rows = names(&core);
    for row in [
        (WORK, "Alex", Some("alex@example.com")),
        (HOME, "Work", Some("alex.work@Company.Example")),
        (TYPED, "jane@company.example", Some("jane@company.example")),
        (UNSIGNED, "Your Chrome", None),
    ] {
        let row = (row.0.into(), row.1.into(), row.2.map(str::to_owned));
        assert!(rows.contains(&row), "{rows:?}");
    }
    let chrome = &core.shared.chrome_control;
    let expected = [
        (WORK, "Chrome · Alex (example.com)"),
        (HOME, "Chrome · Work (company.example)"),
        (MANAGED, "Chrome · company.example"),
        (TYPED, "Chrome · jane@company.example"),
        (UNSIGNED, "Chrome · Your Chrome"),
    ];
    for (id, label) in expected {
        assert_eq!(super::profiles::label(chrome, id), label);
        let name = label.trim_start_matches("Chrome · ");
        assert_eq!(chrome.profiles.lock().unwrap().agent_name(id), name);
    }
    let mut texts = Vec::new();
    let mut result = json!({"structuredContent":{},"content":[]});
    super::profile_agent::status(&core.shared, "s", &mut result);
    let status = result.to_string();
    for (_, label) in expected {
        let name = label.trim_start_matches("Chrome · ");
        assert!(status.contains(&format!("\"name\":\"{name}\"")), "{status}");
    }
    texts.push(status);
    for id in [WORK, HOME, MANAGED, UNSIGNED] {
        texts.push(super::profile_agent::unavailable(&core.shared, id));
        texts.push(super::profile_presence::removed_message(chrome, id));
    }
    assert!(
        texts[1].starts_with("Chrome · Alex (example.com) is unavailable"),
        "{}",
        texts[1]
    );
    let mut inventory =
        json!({"structuredContent":{"profiles":[{"profileId":WORK},{"profileId":HOME}]}});
    super::profile_agent::names(chrome, &mut inventory);
    let inventory = inventory.to_string();
    assert!(
        inventory.contains("Alex (example.com)") && inventory.contains("Work (company.example)"),
        "{inventory}"
    );
    texts.push(inventory);
    let mut receipt = json!({"structuredContent":{}});
    super::receipt::stamp(
        &core.shared,
        "chrome_session",
        &json!({"action":"open","profileId":WORK}),
        &mut receipt,
    );
    let receipt = receipt.to_string();
    assert!(receipt.contains("Alex (example.com)"), "{receipt}");
    texts.push(receipt);
    let choices =
        super::profile_seeding::choice_needed(&core.shared, "s", &json!({"profileId":"chrome"}))
            .unwrap_or_default();
    assert_eq!(
        choices.len(),
        5,
        "five profiles and no default ask the person"
    );
    // The card shows the browser's name and the account to the person; the agent's text about the card uses the
    // agent name.
    let work = choices.iter().find(|c| c.profile_id == WORK).unwrap();
    assert_eq!(work.name, "Alex");
    assert_eq!(work.account.as_deref(), Some("alex@example.com"));
    assert_eq!(work.agent_name, "Alex (example.com)");
    texts.extend(choices.iter().map(|c| c.agent_name.clone()));
    // No agent text carries an account; the only email is the name the person typed for TYPED, which is not used in
    // these texts except its own choice.
    for text in &texts {
        let lower = text.to_lowercase();
        assert!(
            !lower.contains("alex@example")
                && !lower.contains("alex.work@")
                && !lower.contains("alex example"),
            "{text}"
        );
        assert!(
            !lower.contains("chrome profile"),
            "a named profile is never numbered: {text}"
        );
    }
    core.shutdown().await;
}

/// A profile without any readable name keeps the numbered name, with its account domain when Chrome lists one; a
/// managed profile whose policy sets an `enterprise_label` is named by that label, as Chrome's picker names it.
///
/// PREDICT (written before the change): fails. The numbered name carries no domain, and `parse` reads `name` before
/// `enterprise_label`, so the labelled profile is called "Person 1".
#[tokio::test]
async fn an_unnamed_profile_keeps_its_number_and_a_policy_label_is_the_name() {
    let core = fixture("names-agent-number", json!([])).await;
    browser_data(
        &core,
        &[("Profile 3", WORK_UUID), ("Profile 1", HOME_UUID)],
        &json!({
            "Profile 3":{"name":"","gaia_name":"","user_name":"sam@example.org"},
            "Profile 1":{"name":"Person 1","enterprise_label":"Acme work","user_name":"a@acme.example",
                "hosted_domain":"acme.example"}
        }),
    );
    observe(&core, &[HOME, WORK]);
    let chrome = &core.shared.chrome_control;
    assert_eq!(
        super::profiles::label(chrome, WORK),
        "Chrome profile 2 (example.org)"
    );
    assert_eq!(
        super::profiles::label(chrome, HOME),
        "Chrome · Acme work (acme.example)"
    );
    core.shutdown().await;
}

const THIRD: &str = "chrome:9a3c1e2f-5b6d-4e7f-8a9b-0c1d2e3f4a5b";

/// PREDICT: fails before the change. Same-named profiles were told apart by their pairing number in brackets
/// ("Work (1)", "Work (3)"), which a person cannot match to anything, and the fallback "Chrome profile N" kept the
/// pairing number after an earlier profile was removed ("Chrome profile 1", "Chrome profile 3"). After the change
/// same-named profiles carry when each was paired, and fallback numbers count only the profiles shown, without gaps.
#[tokio::test]
async fn same_named_profiles_show_when_they_were_paired_not_a_bare_number() {
    let core = fixture("names-paired-at", json!([])).await;
    observe(&core, &[WORK, HOME]);
    {
        let mut saved = core.shared.chrome_control.profiles.lock().unwrap();
        for (id, at) in [(WORK, 1_762_000_000_000_u64), (HOME, 1_762_900_000_000)] {
            let record = saved.records.get_mut(id).unwrap();
            record.browser_name = Some("Work".into());
            record.paired_at = Some(at);
        }
    }
    let rows = names(&core);
    let labels: Vec<_> = rows.iter().map(|row| row.1.clone()).collect();
    assert_ne!(labels[0], labels[1], "{labels:?}");
    for label in &labels {
        assert!(label.starts_with("Work — paired "), "{labels:?}");
        assert!(!label.contains('('), "{labels:?}");
    }
    // Agent text carries the same name (it has no email), never the account.
    let agent = super::profiles::label(&core.shared.chrome_control, WORK);
    assert!(agent.contains("paired") && !agent.contains('@'), "{agent}");
    core.shutdown().await;
}

#[tokio::test]
async fn fallback_numbers_stay_with_their_profile_when_another_is_removed() {
    let core = fixture("names-no-gaps", json!([])).await;
    observe(&core, &[HOME]);
    observe(&core, &[HOME, WORK]);
    observe(&core, &[HOME, WORK, THIRD]);
    core.shared
        .chrome_control
        .profiles
        .lock()
        .unwrap()
        .records
        .get_mut(WORK)
        .unwrap()
        .removed = true;
    let labels: Vec<_> = names(&core).into_iter().map(|row| row.1).collect();
    assert_eq!(labels, ["Chrome profile 1", "Chrome profile 3"]);
    core.shutdown().await;
}

/// Same-named profiles signed in to different accounts are told apart by the account the screen already shows next to
/// the name, not by when each was paired; the pairing time stays only when nothing else differs. Agent text, which
/// carries the domain and never the account, adds the pairing time only when name and domain are both equal.
///
/// PREDICT (written before the change): fails. `name` compares the browser's names only, so two "Work" profiles with
/// accounts a@one.example and b@two.example are shown as "Work — paired …" on the card and in Settings although
/// their accounts differ, and agent text "Work (one.example) — paired …" carries the time although the domains differ.
#[tokio::test]
async fn same_named_profiles_with_different_accounts_are_told_apart_by_the_account() {
    let core = fixture("names-twins-accounts", json!([])).await;
    observe(&core, &[WORK, HOME, THIRD]);
    let set = |accounts: [Option<&str>; 3]| {
        let mut saved = core.shared.chrome_control.profiles.lock().unwrap();
        for ((id, at), account) in [
            (WORK, 1_762_000_000_000_u64),
            (HOME, 1_762_900_000_000),
            (THIRD, 1_763_000_000_000),
        ]
        .into_iter()
        .zip(accounts)
        {
            let record = saved.records.get_mut(id).unwrap();
            record.browser_name = Some("Work".into());
            record.paired_at = Some(at);
            record.account = account.map(str::to_owned);
        }
    };
    let screen = |id: &str| core.shared.chrome_control.profiles.lock().unwrap().name(id);
    let agent = |id: &str| {
        core.shared
            .chrome_control
            .profiles
            .lock()
            .unwrap()
            .agent_name(id)
    };
    // Three different accounts, two sharing a domain.
    set([
        Some("a@one.example"),
        Some("b@two.example"),
        Some("c@two.example"),
    ]);
    for id in [WORK, HOME, THIRD] {
        assert_eq!(screen(id), "Work", "the account tells them apart");
    }
    assert_eq!(agent(WORK), "Work (one.example)");
    assert!(
        agent(HOME).starts_with("Work (two.example) — paired "),
        "{}",
        agent(HOME)
    );
    assert!(
        agent(THIRD).starts_with("Work (two.example) — paired "),
        "{}",
        agent(THIRD)
    );
    assert_ne!(agent(HOME), agent(THIRD));
    // Same account (or none on both): only the pairing time differs, so it stays.
    set([Some("a@one.example"), Some("a@one.example"), None]);
    assert!(
        screen(WORK).starts_with("Work — paired "),
        "{}",
        screen(WORK)
    );
    assert!(
        screen(HOME).starts_with("Work — paired "),
        "{}",
        screen(HOME)
    );
    assert_ne!(screen(WORK), screen(HOME));
    assert_eq!(screen(THIRD), "Work", "no account differs from an account");
    core.shutdown().await;
}
