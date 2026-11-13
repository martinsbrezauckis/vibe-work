//! A Safari pairing the person approves replaces the Safari identities that are not connected at that moment.
//!
//! PREDICT (written before the change): every test here fails on the base. A Safari identity is a UUID the extension
//! keeps in its own storage; when that storage starts empty again the same Safari pairs under a new identity, and the
//! old record stays as an Offline profile named like the new one. Settings then lists "Safari (1)" and "Safari (3)",
//! and an unbound chat gets a card with both (`choice_needed` returns two choices). After the change the approved
//! pairing marks the other Safari identities that are not live as superseded: they leave Settings and the card, their
//! pin and record are kept, and a live connection of one of them (proof that it is a different, running Safari
//! profile) brings it back. Two identities that are both connected are never folded together.
//!
//! Uses the real ask service with a controlled native presenter and an owner pipe that answers every request.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test fixtures and held calls must exist for the test to mean anything"
)]
use std::{sync::Arc, time::Duration};

use serde_json::json;
use tokio::sync::{mpsc, oneshot};
use vw_gate::{Confirmation, ConfirmationRequest, Confirmer};

const OLD: &str = "safari:e9d88e13-473b-4586-8784-7b5eb7f1989f";
const NEW: &str = "safari:09cd8b03-0ab5-4f21-9e2b-850ae4493d9f";
const REQUEST: &str = "11111111-1111-4111-8111-111111111111";

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

async fn core(name: &str) -> (crate::Core, Prompts) {
    let (send, receive) = mpsc::unbounded_channel();
    let core = crate::offline_core::start_with_confirmer(
        name,
        |config, dir| {
            config.chrome_control.package = Some(dir.join("package"));
        },
        Presenter(send),
    )
    .await;
    let dir = core.shared.chrome_control.dir().unwrap();
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("safari-enabled.json"), b"true").unwrap();
    core.shared
        .chrome_control
        .owner
        .answering(|_, _| json!({"approved":true}));
    super::test_support::chat(&core.shared, "s");
    (core, receive)
}

fn status(core: &crate::Core, rows: &[(&str, bool)]) {
    let rows: Vec<_> = rows
        .iter()
        .map(|(id, connected)| json!({"profileId":id,"connected":connected,"name":"Safari"}))
        .collect();
    super::test_support::observe_status(&core.shared, &json!({ "profiles": rows }));
}

/// The person approves the native "Connect Safari to Vibe Work?" dialog for `profile`.
async fn pair(core: &crate::Core, prompts: &mut Prompts, profile: &str) {
    super::owner::observe(
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
    tokio::time::sleep(Duration::from_millis(100)).await;
}

fn safari_rows(core: &crate::Core) -> Vec<super::profiles::BrowserProfile> {
    super::profiles::view(&core.shared)
        .into_iter()
        .filter(|row| row.browser == "safari")
        .collect()
}

fn card_choices(core: &crate::Core, chat: &str) -> Option<Vec<String>> {
    super::profile_seeding::choice_needed(&core.shared, chat, &json!({"profileId":"safari"}))
        .map(|choices| choices.into_iter().map(|row| row.profile_id).collect())
}

#[tokio::test]
async fn pairing_safari_again_leaves_one_safari_and_no_card() {
    let (core, mut prompts) = core("safari-supersede").await;
    // The first pairing: connected once, then Safari's extension storage starts empty again (the old identity is gone
    // from Safari for good), so it never connects again.
    status(&core, &[(OLD, true)]);
    status(&core, &[(OLD, false)]);
    // The same Safari pairs again under a new identity.
    pair(&core, &mut prompts, NEW).await;
    status(&core, &[(OLD, false), (NEW, true)]);
    let rows = safari_rows(&core);
    assert_eq!(
        rows.iter()
            .map(|row| row.profile_id.as_str())
            .collect::<Vec<_>>(),
        [NEW],
        "Settings shows one Safari: {rows:?}"
    );
    assert_eq!(rows[0].name, "Safari", "{rows:?}");
    assert_eq!(card_choices(&core, "s"), None, "one Safari needs no card");
    // The old identity's record is kept (not merged into the new one, not deleted).
    assert!(
        core.shared
            .chrome_control
            .profiles
            .lock()
            .unwrap()
            .records
            .contains_key(OLD)
    );
    core.shutdown().await;
}

#[tokio::test]
async fn a_superseded_identity_that_connects_again_is_its_own_profile() {
    let (core, mut prompts) = core("safari-supersede-back").await;
    status(&core, &[(OLD, false)]);
    pair(&core, &mut prompts, NEW).await;
    status(&core, &[(OLD, false), (NEW, true)]);
    assert_eq!(safari_rows(&core).len(), 1);
    // The old pairing was made a day earlier.
    if let Some(record) = core
        .shared
        .chrome_control
        .profiles
        .lock()
        .unwrap()
        .records
        .get_mut(OLD)
    {
        record.paired_at = record.paired_at.map(|at| at - 86_400_000);
    }
    // A live connection proves the old identity is a different running Safari profile: it comes back.
    status(&core, &[(OLD, true), (NEW, true)]);
    let rows = safari_rows(&core);
    assert_eq!(rows.len(), 2, "{rows:?}");
    let names: Vec<_> = rows.iter().map(|row| row.name.clone()).collect();
    assert_ne!(names[0], names[1], "{names:?}");
    for name in &names {
        assert!(
            !name.chars().any(|c| c.is_ascii_digit()) || name.contains("paired"),
            "a bare number names a Safari profile: {names:?}"
        );
        assert!(!name.contains('('), "{names:?}");
    }
    let mut offered = card_choices(&core, "s").unwrap();
    offered.sort();
    let mut both = vec![NEW.to_owned(), OLD.to_owned()];
    both.sort();
    assert_eq!(offered, both);
    core.shutdown().await;
}

#[tokio::test]
async fn two_connected_safari_profiles_are_never_folded_together() {
    let (core, mut prompts) = core("safari-two-live").await;
    // OLD is live while NEW pairs: two Safari profiles are running, so neither replaces the other.
    status(&core, &[(OLD, true)]);
    pair(&core, &mut prompts, NEW).await;
    status(&core, &[(OLD, true), (NEW, true)]);
    assert_eq!(safari_rows(&core).len(), 2);
    core.shutdown().await;
}

#[tokio::test]
async fn a_chat_bound_to_the_replaced_identity_asks_before_it_moves() {
    let (core, mut prompts) = core("safari-supersede-bound").await;
    status(&core, &[(OLD, true)]);
    core.shared
        .chrome_control
        .chat_profiles
        .lock()
        .unwrap()
        .insert("s".into(), OLD.into());
    status(&core, &[(OLD, false)]);
    pair(&core, &mut prompts, NEW).await;
    status(&core, &[(OLD, false), (NEW, true)]);
    // The chat's own profile is gone from Safari; the person confirms the move on the card, never silently.
    assert_eq!(card_choices(&core, "s"), Some(vec![NEW.to_owned()]));
    core.shutdown().await;
}
