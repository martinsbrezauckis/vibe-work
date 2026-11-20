//! How a paired profile is labelled for the agent and the screen: the signed-in account's domain in agent text, a
//! policy label as the name, numbered fallbacks without gaps, and same-named profiles told apart by account or by
//! when each was paired.
#![allow(
    clippy::unwrap_used,
    reason = "temporary browser folders and fixtures must be created for the test to mean anything"
)]
use super::edge_agent_tests::fixture;
use super::profile_names_tests::{HOME, HOME_UUID, WORK, WORK_UUID, browser_data, names, observe};
use serde_json::json;

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
    // PREDICT (bracket form): red before the change, which wrote "Work — paired 12 Nov 23:25"; UI text has no em dash.
    for label in &labels {
        assert!(label.starts_with("Work (paired "), "{labels:?}");
        assert!(
            label.ends_with(')') && !label.contains(['—', ',']),
            "{labels:?}"
        );
    }
    // Identical pairing times fall back to their order inside the same brackets.
    for id in [WORK, HOME] {
        let mut saved = core.shared.chrome_control.profiles.lock().unwrap();
        saved.records.get_mut(id).unwrap().paired_at = Some(1_762_000_000_000);
    }
    let same: Vec<_> = names(&core).into_iter().map(|row| row.1).collect();
    assert_ne!(same[0], same[1], "{same:?}");
    for label in &same {
        assert!(label.starts_with("Work (paired "), "{same:?}");
        assert!(
            (label.ends_with(", 1)") || label.ends_with(", 2)")) && !label.contains('—'),
            "{same:?}"
        );
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
        agent(HOME).starts_with("Work (two.example) (paired "),
        "{}",
        agent(HOME)
    );
    assert!(
        agent(THIRD).starts_with("Work (two.example) (paired "),
        "{}",
        agent(THIRD)
    );
    assert_ne!(agent(HOME), agent(THIRD));
    // Same account (or none on both): only the pairing time differs, so it stays.
    set([Some("a@one.example"), Some("a@one.example"), None]);
    assert!(
        screen(WORK).starts_with("Work (paired "),
        "{}",
        screen(WORK)
    );
    assert!(
        screen(HOME).starts_with("Work (paired "),
        "{}",
        screen(HOME)
    );
    assert_ne!(screen(WORK), screen(HOME));
    assert_eq!(screen(THIRD), "Work", "no account differs from an account");
    core.shutdown().await;
}
