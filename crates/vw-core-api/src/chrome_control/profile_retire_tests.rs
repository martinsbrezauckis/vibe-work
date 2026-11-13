//! Settings and the profile card list each real browser profile once, under a name a person recognises.
//!
//! PREDICT (written before the change): every test here fails on the base.
//! - The helper registers every Safari and Firefox extension under the browser's product name ("Safari", "Mozilla
//!   Firefox"); the base took that as the profile's name, so Firefox showed as "Mozilla Firefox", agent text said
//!   "Firefox · Mozilla Firefox", and two Safari identities became "Safari (1)" / "Safari (2)".
//! - The owned agent Firefox window reports as `firefox:agent`; the base kept it as a paired profile, so it was a row
//!   in Settings and a second button on the Firefox card.
//! - A reinstalled Firefox extension pairs under a new identity; the base kept the old one as an Offline twin.
//! - A newer Chrome identity proven in the same profile folder as an older one (the extension was reinstalled in that
//!   profile) left both rows, named alike.
//! - Removing Firefox in Settings left its identities in the inventory; after Connect again they came back Offline.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test fixtures and held calls must exist for the test to mean anything"
)]
use serde_json::json;

const CHROME_OLD: &str = "chrome:4c2d7e8a-1b3f-4a5c-9d6e-7f8091a2b3c4";
const CHROME_NEW: &str = "chrome:8e9f0a1b-2c3d-4e5f-8a6b-7c8d9e0f1a2b";

async fn core(name: &str) -> crate::Core {
    let core = crate::offline_core::start_with(name, |config, dir| {
        config.chrome_control.package = Some(dir.join("package"));
    })
    .await;
    let dir = core.shared.chrome_control.dir().unwrap();
    std::fs::create_dir_all(dir).unwrap();
    super::test_support::chat(&core.shared, "s");
    core
}

fn status(core: &crate::Core, rows: &[(&str, bool, &str)]) {
    let rows: Vec<_> = rows
        .iter()
        .map(|(id, connected, name)| json!({"profileId":id,"connected":connected,"name":name}))
        .collect();
    super::test_support::observe_status(&core.shared, &json!({ "profiles": rows }));
}

fn rows(core: &crate::Core, browser: &str) -> Vec<(String, String)> {
    super::profiles::view(&core.shared)
        .into_iter()
        .filter(|row| row.browser == browser)
        .map(|row| (row.profile_id, row.name))
        .collect()
}

fn card(core: &crate::Core, browser: &str) -> Option<Vec<String>> {
    super::profile_seeding::choice_needed(&core.shared, "s", &json!({ "profileId": browser }))
        .map(|choices| choices.into_iter().map(|row| row.profile_id).collect())
}

fn record(core: &crate::Core, id: &str, change: impl FnOnce(&mut super::profiles::Record)) {
    change(
        core.shared
            .chrome_control
            .profiles
            .lock()
            .unwrap()
            .records
            .get_mut(id)
            .unwrap(),
    );
}

#[tokio::test]
async fn registration_names_never_become_profile_names() {
    let core = core("retire-registration-names").await;
    for name in [
        "Safari",
        "Mozilla Firefox",
        "Firefox",
        "Your Safari",
        "Google Chrome",
    ] {
        status(&core, &[(CHROME_OLD, true, name)]);
        assert_eq!(
            rows(&core, "chrome"),
            [(CHROME_OLD.into(), "Chrome profile 1".into())],
            "{name}"
        );
    }
    status(&core, &[(CHROME_OLD, true, "Work")]);
    assert_eq!(rows(&core, "chrome"), [(CHROME_OLD.into(), "Work".into())]);
    core.shutdown().await;
}

#[cfg(feature = "your-safari")]
mod safari {
    use super::*;
    const ONE: &str = "safari:e9d88e13-473b-4586-8784-7b5eb7f1989f";
    const TWO: &str = "safari:09cd8b03-0ab5-4f21-9e2b-850ae4493d9f";

    #[tokio::test]
    async fn a_safari_profile_is_named_safari_and_two_carry_their_pairing_time() {
        let core = core("retire-safari-names").await;
        std::fs::write(
            core.shared
                .chrome_control
                .dir()
                .unwrap()
                .join("safari-enabled.json"),
            b"true",
        )
        .unwrap();
        status(&core, &[(ONE, true, "Safari")]);
        assert_eq!(rows(&core, "safari"), [(ONE.into(), "Safari".into())]);
        assert_eq!(
            super::super::profiles::label(&core.shared.chrome_control, ONE),
            "Safari"
        );
        // A second Safari profile that is really running: both shown, told apart by pairing time, no bare number.
        status(&core, &[(ONE, true, "Safari"), (TWO, true, "Safari")]);
        record(&core, ONE, |record| {
            record.paired_at = Some(1_762_000_000_000);
        });
        record(&core, TWO, |record| {
            record.paired_at = Some(1_762_900_000_000);
        });
        let names: Vec<_> = rows(&core, "safari").into_iter().map(|row| row.1).collect();
        assert_eq!(names.len(), 2, "{names:?}");
        assert_ne!(names[0], names[1], "{names:?}");
        for name in &names {
            assert!(name.starts_with("Safari — paired "), "{names:?}");
            assert!(!name.contains('('), "{names:?}");
        }
        // A name saved by an earlier version from the registration is ignored as well.
        record(&core, ONE, |record| {
            record.metadata_name = Some("Safari".into());
        });
        let label = super::super::profiles::label(&core.shared.chrome_control, ONE);
        assert!(label.starts_with("Safari — paired "), "{label}");
        core.shutdown().await;
    }
}

#[cfg(feature = "your-firefox")]
mod firefox {
    use super::*;
    use std::{sync::Arc, time::Duration};
    use tokio::sync::{mpsc, oneshot};
    use vw_gate::{Confirmation, ConfirmationRequest, Confirmer};

    const OLD: &str = "firefox:2f1e0d9c-8b7a-4654-8321-0fedcba98765";
    const NEW: &str = "firefox:a1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d";
    const REQUEST: &str = "22222222-2222-4222-8222-222222222222";

    type Prompts = mpsc::UnboundedReceiver<(ConfirmationRequest, oneshot::Sender<Confirmation>)>;
    struct Presenter(mpsc::UnboundedSender<(ConfirmationRequest, oneshot::Sender<Confirmation>)>);
    impl Confirmer for Presenter {
        fn confirm(
            &self,
            request: &ConfirmationRequest,
        ) -> impl std::future::Future<Output = Confirmation> + Send {
            let (send, receive) = oneshot::channel();
            self.0.send((request.clone(), send)).unwrap();
            async move { receive.await.unwrap_or(Confirmation::Declined) }
        }
    }

    async fn paired_core(name: &str) -> (crate::Core, Prompts) {
        let (send, receive) = mpsc::unbounded_channel();
        let core = crate::offline_core::start_with_confirmer(
            name,
            |config, dir| {
                config.chrome_control.package = Some(dir.join("package"));
            },
            Presenter(send),
        )
        .await;
        std::fs::create_dir_all(core.shared.chrome_control.dir().unwrap()).unwrap();
        core.shared
            .chrome_control
            .owner
            .answering(|name, _| match name {
                "chrome_status" => json!({"structuredContent":{"firefoxWindow":"Window closed"}}),
                "owner.browserUnpair" => json!({"unpaired":true}),
                "owner.browserAvailability" => json!({"applied":true}),
                _ => json!({"approved":true}),
            });
        super::super::test_support::chat(&core.shared, "s");
        (core, receive)
    }

    async fn pair(core: &crate::Core, prompts: &mut Prompts, profile: &str) {
        super::super::owner::observe(
            &Arc::downgrade(&core.shared),
            &json!({"method":"notifications/owner/pairingApproval",
                "params":{"requestId":REQUEST,"profileId":profile}}),
        )
        .unwrap();
        let (_, answer) = tokio::time::timeout(Duration::from_secs(2), prompts.recv())
            .await
            .expect("pairing must reach the native presenter")
            .unwrap();
        answer.send(Confirmation::Approved).unwrap();
        // The approval task records the new identity as paired after the helper's answer; wait for that record (only
        // that task writes it before the next status) instead of a fixed time, which a busy test run can outlast.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while core
            .shared
            .chrome_control
            .profiles
            .lock()
            .unwrap()
            .records
            .get(profile)
            .is_none_or(|record| record.paired_at.is_none())
        {
            assert!(
                tokio::time::Instant::now() < deadline,
                "the approved pairing must be recorded"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    #[tokio::test]
    async fn a_firefox_profile_is_named_firefox_not_the_registration_name() {
        let core = core("retire-firefox-names").await;
        status(&core, &[(OLD, true, "Mozilla Firefox")]);
        assert_eq!(rows(&core, "firefox"), [(OLD.into(), "Firefox".into())]);
        assert_eq!(
            super::super::profiles::label(&core.shared.chrome_control, OLD),
            "Firefox"
        );
        core.shutdown().await;
    }

    #[tokio::test]
    async fn the_owned_agent_firefox_window_is_never_a_profile_or_a_card_choice() {
        let core = core("retire-firefox-agent").await;
        status(
            &core,
            &[
                ("firefox:agent", true, "Mozilla Firefox"),
                (OLD, true, "Mozilla Firefox"),
            ],
        );
        assert_eq!(rows(&core, "firefox"), [(OLD.into(), "Firefox".into())]);
        assert_eq!(
            card(&core, "firefox"),
            None,
            "one usable Firefox needs no card"
        );
        core.shutdown().await;
    }

    #[tokio::test]
    async fn pairing_firefox_again_leaves_one_firefox_and_no_card() {
        let (core, mut prompts) = paired_core("retire-firefox-repair").await;
        status(&core, &[(OLD, true, "Mozilla Firefox")]);
        status(&core, &[(OLD, false, "Mozilla Firefox")]);
        pair(&core, &mut prompts, NEW).await;
        status(
            &core,
            &[
                (OLD, false, "Mozilla Firefox"),
                (NEW, true, "Mozilla Firefox"),
            ],
        );
        assert_eq!(rows(&core, "firefox"), [(NEW.into(), "Firefox".into())]);
        assert_eq!(card(&core, "firefox"), None);
        core.shutdown().await;
    }

    #[tokio::test]
    async fn removing_firefox_retires_its_profiles_for_good() {
        let (core, _prompts) = paired_core("retire-firefox-remove").await;
        status(&core, &[(OLD, true, "Mozilla Firefox")]);
        assert_eq!(rows(&core, "firefox").len(), 1);
        core.firefox_remove().await.unwrap();
        assert_eq!(rows(&core, "firefox"), Vec::<(String, String)>::new());
        // Connect again: the helper still lists the old identity, which never connects again.
        core.shared
            .chrome_control
            .set_browser_enabled(super::super::agent_browser::Browser::Firefox, true)
            .unwrap();
        status(&core, &[(OLD, false, "Mozilla Firefox")]);
        assert_eq!(rows(&core, "firefox"), Vec::<(String, String)>::new());
        core.shutdown().await;
    }
}

#[tokio::test]
async fn a_newer_chrome_identity_in_the_same_folder_retires_the_older_one() {
    let core = core("retire-chrome-folder").await;
    status(&core, &[(CHROME_OLD, true, ""), (CHROME_NEW, true, "")]);
    let folder = std::path::PathBuf::from("/fixture/Chrome/Profile 3");
    for id in [CHROME_OLD, CHROME_NEW] {
        record(&core, id, |record| {
            record.folder = Some(folder.clone());
            record.browser_name = Some("Work".into());
        });
    }
    // The reinstalled extension in that folder holds the new identity; the old one never connects again.
    status(&core, &[(CHROME_OLD, false, ""), (CHROME_NEW, true, "")]);
    assert_eq!(rows(&core, "chrome"), [(CHROME_NEW.into(), "Work".into())]);
    assert_eq!(card(&core, "chrome"), None);
    // A different folder is a different profile: both stay.
    record(&core, CHROME_OLD, |record| {
        record.folder = Some("/fixture/Chrome/Profile 4".into());
    });
    assert_eq!(rows(&core, "chrome").len(), 2);
    core.shutdown().await;
}
