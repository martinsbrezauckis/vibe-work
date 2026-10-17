//! End-to-end GTK fixture through real AT-SPI Action, `EditableText`, Component and fresh reads.
//! Depends on Xvfb, session D-Bus and GTK 3/PyGObject; invoked explicitly by the crate demo runner.

#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use vw_desktop::{Action, Desktop, ElementId, Error, Postcondition, Receipt, Snapshot, State};

struct Fixture(Child);

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn element(snapshot: &Snapshot, name: &str) -> ElementId {
    snapshot
        .elements
        .iter()
        .find(|e| e.name == name)
        .expect("fixture control exported")
        .id
        .clone()
}

async fn wait_for_window(desktop: &Desktop, fixture: &mut Fixture) -> Snapshot {
    let started = Instant::now();
    loop {
        assert!(
            fixture.0.try_wait().unwrap().is_none(),
            "GTK fixture exited"
        );
        match desktop.snapshot(Some("Desktop pilot fixture")).await {
            Ok(snapshot) => return snapshot,
            Err(Error::WindowNotFound) if started.elapsed() < Duration::from_secs(5) => {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            other => panic!("fixture window unavailable: {other:?}"),
        }
    }
}

async fn verify_controls(desktop: &Desktop, before: &Snapshot) -> Receipt {
    let button = element(before, "Press me");
    let entry = element(before, "Message");
    let press = desktop
        .act(
            &button,
            Action::Press {
                expected: Postcondition::Name("Pressed 1".into()),
            },
        )
        .await
        .unwrap();
    assert!(press.is_success(), "button effect verified: {press:?}");
    assert_eq!(press.verification.observed.id, button);
    assert!(
        desktop
            .verify(&button, &Postcondition::Name("Pressed 1".into()))
            .await
            .unwrap()
            .matched
    );

    let text = desktop
        .act(&entry, Action::SetText("Rīga 🦀 via AT-SPI".into()))
        .await
        .unwrap();
    assert!(text.is_success(), "exact Unicode text verified: {text:?}");
    assert_eq!(
        text.verification.text.as_ref().unwrap().value,
        "Rīga 🦀 via AT-SPI"
    );
    let focus = desktop.act(&entry, Action::Focus).await.unwrap();
    assert!(focus.is_success(), "keyboard focus verified: {focus:?}");
    assert!(focus.verification.observed.state.contains(State::Focused));
    text
}

async fn verify_unmet_controls(desktop: &Desktop, before: &Snapshot) -> Receipt {
    let ignored = desktop
        .act(
            &element(before, "Ignore press"),
            Action::Press {
                expected: Postcondition::Name("Changed".into()),
            },
        )
        .await
        .unwrap();
    assert!(ignored.accepted);
    assert!(
        !ignored.is_success(),
        "accepted default action without requested effect must fail"
    );
    let reverted = desktop
        .act(
            &element(before, "Reverts text"),
            Action::SetText("discarded".into()),
        )
        .await
        .unwrap();
    assert!(reverted.accepted);
    assert!(
        !reverted.is_success(),
        "provider accepted text but the field reverted it"
    );
    assert_eq!(reverted.verification.text.as_ref().unwrap().value, "kept");

    let label = element(before, "Read only label");
    assert!(matches!(
        desktop
            .act(
                &label,
                Action::Press {
                    expected: Postcondition::Name("Changed".into()),
                }
            )
            .await,
        Err(Error::Unsupported("Action"))
    ));
    reverted
}

#[tokio::test]
#[ignore = "requires an isolated Xvfb/session D-Bus; run tests/run-demo.sh"]
async fn gtk_observe_act_verify() {
    let desktop = match std::env::var("VW_DESKTOP_TEST_BUS") {
        Ok(address) => Desktop::connect_at(&address).await,
        Err(_) => Desktop::connect().await,
    }
    .expect("session and accessibility buses available");
    let mut fixture = Fixture(
        Command::new("python3")
            .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/gtk_fixture.py"))
            .stdout(Stdio::null())
            .spawn()
            .expect("GTK fixture starts"),
    );
    let before = wait_for_window(&desktop, &mut fixture).await;
    assert!(!before.truncated);
    assert!(before.elements.iter().all(|e| e.bounds.is_some()));
    let focused = desktop
        .snapshot(None)
        .await
        .expect("GTK fixture is the focused window");
    assert_eq!(before.window, focused.window);

    let text = verify_controls(&desktop, &before).await;
    let reverted = verify_unmet_controls(&desktop, &before).await;
    let after = desktop
        .snapshot(Some("Desktop pilot fixture"))
        .await
        .unwrap();
    assert_eq!(
        element(&after, "Pressed 1"),
        element(&before, "Press me"),
        "press was dispatched only once"
    );
    assert_eq!(element(&after, "Message"), element(&before, "Message"));
    // Capture this JSON line as fixture evidence; recording never changes the test's behavior.
    println!(
        "RECORDING {}",
        serde_json::json!({
            "_comment": "Recorded GTK 3 AT-SPI observations; parsed and replayed by the crate unit tests.",
            "snapshot": after, "text": text, "reverted": reverted,
        })
    );

    fixture.0.kill().unwrap();
    fixture.0.wait().unwrap();
    assert!(
        desktop
            .verify(
                &element(&before, "Message"),
                &Postcondition::Text("Rīga 🦀 via AT-SPI".into())
            )
            .await
            .is_err(),
        "a closed provider cannot report verified success"
    );
}
