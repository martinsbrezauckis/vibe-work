//! Deterministic accessibility-provider falsifiers, independent of D-Bus and a display.
//! Depends on the production engine and serde; run by the crate's unit-test harness.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::cell::{Cell, RefCell};

use crate::engine::{Backend, Engine, Mutation};
use crate::model::*;
use crate::{Role, State, StateSet};

struct Node {
    element: Element,
    children: Vec<ElementId>,
    text: TextValue,
}

#[derive(Clone, Copy)]
enum Behavior {
    Change,
    Ignore,
    Reject,
}

struct Fixture {
    nodes: RefCell<Vec<Node>>,
    behavior: Cell<Behavior>,
    mutations: Cell<usize>,
    reads: Cell<usize>,
    verification_error: Cell<bool>,
}

fn id(index: usize) -> ElementId {
    ElementId {
        bus: ":1.42".try_into().unwrap(),
        path: format!("/org/a11y/atspi/accessible/{index}")
            .try_into()
            .unwrap(),
    }
}

fn node(index: usize, role: Role, name: &str, states: &[State], children: &[usize]) -> Node {
    Node {
        element: Element {
            id: id(index),
            role,
            name: name.into(),
            name_truncated: false,
            state: states.iter().copied().collect::<StateSet>(),
            bounds: Some(Bounds {
                x: 10,
                y: 20,
                width: 120,
                height: 32,
            }),
            depth: 0,
        },
        children: children.iter().map(|i| id(*i)).collect(),
        text: TextValue {
            value: String::new(),
            truncated: false,
        },
    }
}

fn fixture() -> Engine<Fixture> {
    Engine(Fixture {
        nodes: RefCell::new(vec![
            node(0, Role::DesktopFrame, "main", &[], &[1]),
            node(1, Role::Application, "fixture", &[], &[2]),
            node(2, Role::Frame, "Pilot window", &[State::Active], &[3, 4]),
            node(
                3,
                Role::Button,
                "Press me",
                &[State::Enabled, State::Sensitive, State::Focusable],
                &[],
            ),
            node(
                4,
                Role::Entry,
                "Message",
                &[
                    State::Enabled,
                    State::Sensitive,
                    State::Editable,
                    State::Focusable,
                ],
                &[],
            ),
        ]),
        behavior: Cell::new(Behavior::Change),
        mutations: Cell::new(0),
        reads: Cell::new(0),
        verification_error: Cell::new(false),
    })
}

// These synchronous fixture operations implement the same async contract as the real bus.
#[allow(clippy::unused_async_trait_impl)]
impl Backend for Fixture {
    async fn root(&self) -> Result<ElementId, Error> {
        Ok(id(0))
    }

    async fn read(&self, target: &ElementId) -> Result<Element, Error> {
        self.reads.set(self.reads.get() + 1);
        if self.verification_error.get() && self.mutations.get() > 0 {
            return Err(Error::MissingElement);
        }
        self.nodes
            .borrow()
            .iter()
            .find(|n| n.element.id == *target)
            .map(|n| n.element.clone())
            .ok_or(Error::MissingElement)
    }

    async fn child_count(&self, target: &ElementId) -> Result<i32, Error> {
        let nodes = self.nodes.borrow();
        let n = nodes
            .iter()
            .find(|n| n.element.id == *target)
            .ok_or(Error::MissingElement)?;
        Ok(i32::try_from(n.children.len()).unwrap())
    }

    async fn child(&self, target: &ElementId, index: i32) -> Result<ElementId, Error> {
        let nodes = self.nodes.borrow();
        let n = nodes
            .iter()
            .find(|n| n.element.id == *target)
            .ok_or(Error::MissingElement)?;
        n.children
            .get(usize::try_from(index).unwrap())
            .cloned()
            .ok_or(Error::MissingElement)
    }

    async fn text(&self, target: &ElementId) -> Result<TextValue, Error> {
        self.nodes
            .borrow()
            .iter()
            .find(|n| n.element.id == *target)
            .map(|n| n.text.clone())
            .ok_or(Error::MissingElement)
    }

    async fn mutate(&self, target: &ElementId, mutation: Mutation) -> Result<bool, Error> {
        self.mutations.set(self.mutations.get() + 1);
        let mut nodes = self.nodes.borrow_mut();
        let n = nodes
            .iter_mut()
            .find(|n| n.element.id == *target)
            .ok_or(Error::MissingElement)?;
        match self.behavior.get() {
            Behavior::Change => match mutation {
                Mutation::Press => n.element.name = "Pressed once".into(),
                Mutation::SetText(value) => n.text.value = value,
                Mutation::Focus => n.element.state.insert(State::Focused),
            },
            Behavior::Ignore | Behavior::Reject => {}
        }
        Ok(!matches!(self.behavior.get(), Behavior::Reject))
    }
}

#[tokio::test]
async fn snapshot_selects_named_and_active_window_with_stable_ids() {
    let e = fixture();
    let named = e.snapshot(Some("Pilot window")).await.unwrap();
    assert_eq!(named.window, id(2));
    assert_eq!(
        named
            .elements
            .iter()
            .map(|e| e.id.clone())
            .collect::<Vec<_>>(),
        vec![id(2), id(3), id(4)]
    );
    assert_eq!(
        named.elements.iter().map(|e| e.depth).collect::<Vec<_>>(),
        vec![0, 1, 1]
    );
    assert_eq!(e.snapshot(None).await.unwrap(), named);
    e.0.nodes.borrow_mut()[3].element.name = "Renamed".into();
    e.0.nodes.borrow_mut()[2].children.swap(0, 1);
    let reordered = e.snapshot(None).await.unwrap();
    assert_eq!(reordered.elements[2].id, id(3));
    assert!(!named.truncated);
}

#[tokio::test]
async fn missing_and_ambiguous_windows_are_explicit() {
    let e = fixture();
    assert!(matches!(
        e.snapshot(Some("Missing")).await,
        Err(Error::WindowNotFound)
    ));
    e.0.nodes.borrow_mut()[2].element.state = StateSet::empty();
    assert!(matches!(e.snapshot(None).await, Err(Error::WindowNotFound)));
    e.0.nodes.borrow_mut()[1].children.push(id(5));
    e.0.nodes
        .borrow_mut()
        .push(node(5, Role::Dialog, "Pilot window", &[State::Active], &[]));
    assert!(matches!(
        e.snapshot(Some("Pilot window")).await,
        Err(Error::AmbiguousWindow)
    ));
}

#[tokio::test]
async fn wide_snapshot_and_discovery_are_bounded() {
    let e = fixture();
    e.0.nodes.borrow_mut()[2].children = (5..305).map(id).collect();
    for i in 5..305 {
        e.0.nodes
            .borrow_mut()
            .push(node(i, Role::Label, "Child", &[], &[]));
    }
    let snapshot = e.snapshot(None).await.unwrap();
    assert_eq!(snapshot.elements.len(), MAX_ELEMENTS);
    assert!(snapshot.truncated);
    e.0.nodes.borrow_mut()[1].children = (5..305).map(id).collect();
    assert!(matches!(e.snapshot(None).await, Err(Error::DiscoveryLimit)));
}

#[tokio::test]
async fn depth_and_unicode_names_are_bounded() {
    let e = fixture();
    e.0.nodes.borrow_mut()[2].children = vec![id(5)];
    for i in 5..55 {
        e.0.nodes
            .borrow_mut()
            .push(node(i, Role::Panel, &"界".repeat(200), &[], &[i + 1]));
    }
    e.0.nodes
        .borrow_mut()
        .push(node(55, Role::Label, "End", &[], &[]));
    let snapshot = e.snapshot(None).await.unwrap();
    assert!(snapshot.truncated);
    assert_eq!(snapshot.elements.last().unwrap().depth, MAX_DEPTH);
    assert!(snapshot.elements[1].name.len() <= MAX_NAME_BYTES);
    assert!(snapshot.elements[1].name_truncated);
}

#[tokio::test]
async fn press_text_and_focus_each_mutate_once_and_read_afterward() {
    let e = fixture();
    assert!(
        e.act(
            &id(3),
            Action::Press {
                expected: Postcondition::Name("Pressed once".into())
            }
        )
        .await
        .unwrap()
        .is_success()
    );
    assert!(
        e.act(&id(4), Action::SetText("Rīga 🦀".into()))
            .await
            .unwrap()
            .is_success()
    );
    assert!(e.act(&id(4), Action::Focus).await.unwrap().is_success());
    assert_eq!(e.0.mutations.get(), 3);
    assert!(e.0.reads.get() >= 6);
}

#[tokio::test]
async fn accepted_but_unmet_actions_never_report_success_or_repeat() {
    let e = fixture();
    e.0.behavior.set(Behavior::Ignore);
    let actions = [
        Action::Press {
            expected: Postcondition::Name("Pressed once".into()),
        },
        Action::SetText("new".into()),
        Action::Focus,
    ];
    for action in actions {
        let target = if matches!(action, Action::Press { .. }) {
            id(3)
        } else {
            id(4)
        };
        let receipt = e.act(&target, action).await.unwrap();
        assert!(receipt.accepted);
        assert!(!receipt.verification.matched);
        assert!(!receipt.is_success());
    }
    assert_eq!(e.0.mutations.get(), 3);
}

#[tokio::test]
async fn false_method_reply_is_not_success_even_if_state_already_holds() {
    let e = fixture();
    e.0.behavior.set(Behavior::Reject);
    let receipt = e.act(&id(4), Action::SetText(String::new())).await.unwrap();
    assert!(!receipt.accepted);
    assert!(receipt.verification.matched);
    assert!(!receipt.is_success());
    assert_eq!(e.0.mutations.get(), 1);
}

#[tokio::test]
async fn verification_is_fresh_read_only_and_does_not_prove_truncated_prefixes() {
    let e = fixture();
    let expected = Postcondition::Text("old".into());
    assert!(!e.verify(&id(4), &expected).await.unwrap().matched);
    e.0.nodes.borrow_mut()[4].text.value = "old".into();
    assert!(e.verify(&id(4), &expected).await.unwrap().matched);
    e.0.nodes.borrow_mut()[4].text.truncated = true;
    assert!(!e.verify(&id(4), &expected).await.unwrap().matched);
    e.0.nodes.borrow_mut()[3].element.name_truncated = true;
    assert!(
        !e.verify(&id(3), &Postcondition::Name("Press me".into()))
            .await
            .unwrap()
            .matched
    );
    assert!(
        e.verify(
            &id(4),
            &Postcondition::State {
                state: State::Checked,
                present: false
            }
        )
        .await
        .unwrap()
        .matched
    );
    assert_eq!(e.0.mutations.get(), 0);
}

#[tokio::test]
async fn disabled_targets_and_oversized_requests_do_not_dispatch() {
    let e = fixture();
    e.0.nodes.borrow_mut()[3].element.state = StateSet::empty();
    assert!(matches!(
        e.act(
            &id(3),
            Action::Press {
                expected: Postcondition::Name("Pressed once".into())
            }
        )
        .await,
        Err(Error::NotActionable)
    ));
    assert!(matches!(
        e.act(&id(4), Action::SetText("x".repeat(MAX_TEXT_BYTES + 1)))
            .await,
        Err(Error::SizeLimit(_))
    ));
    assert!(matches!(
        e.verify(&id(3), &Postcondition::Name("x".repeat(MAX_NAME_BYTES + 1)))
            .await,
        Err(Error::SizeLimit(_))
    ));
    assert_eq!(e.0.mutations.get(), 0);
}

#[tokio::test]
async fn vanished_target_after_dispatch_cannot_report_success() {
    let e = fixture();
    e.0.verification_error.set(true);
    assert!(matches!(
        e.act(&id(4), Action::Focus).await,
        Err(Error::MissingElement)
    ));
    assert_eq!(e.0.mutations.get(), 1);
}

#[test]
fn standard_dbus_types_validate_serialized_ids() {
    let valid = serde_json::to_string(&id(4)).unwrap();
    assert_eq!(serde_json::from_str::<ElementId>(&valid).unwrap(), id(4));
    for invalid in [
        r#"{"bus":"application","path":"/valid"}"#,
        r#"{"bus":":1.42","path":"not/a/path"}"#,
        r#"{"bus":":1.42","path":"/invalid-path"}"#,
    ] {
        assert!(serde_json::from_str::<ElementId>(invalid).is_err());
    }
}

#[tokio::test]
async fn recorded_gtk_fixture_parses_and_replays_verification_without_a_bus() {
    #[derive(serde::Deserialize)]
    struct Recording {
        snapshot: Snapshot,
        text: Receipt,
        reverted: Receipt,
    }
    let recording: Recording =
        serde_json::from_str(include_str!("../tests/fixtures/gtk.json")).unwrap();
    assert_eq!(recording.snapshot.elements.len(), 7);
    assert!(!recording.snapshot.truncated);
    assert!(recording.snapshot.elements[0].state.contains(State::Active));
    assert_eq!(recording.snapshot.elements[2].role, Role::Button);
    assert!(recording.text.is_success());
    assert!(!recording.reverted.is_success());
    let e = fixture();
    let records = [&recording.text, &recording.reverted];
    *e.0.nodes.borrow_mut() = recording
        .snapshot
        .elements
        .into_iter()
        .map(|element| {
            let text = records
                .iter()
                .find(|r| r.verification.observed.id == element.id)
                .and_then(|r| r.verification.text.clone())
                .unwrap_or(TextValue {
                    value: String::new(),
                    truncated: false,
                });
            Node {
                element,
                text,
                children: Vec::new(),
            }
        })
        .collect();
    let message = &recording.text.verification.observed.id;
    let reverted = &recording.reverted.verification.observed.id;
    assert!(
        e.verify(message, &Postcondition::Text("Rīga 🦀 via AT-SPI".into()))
            .await
            .unwrap()
            .matched
    );
    assert!(
        !e.verify(reverted, &Postcondition::Text("discarded".into()))
            .await
            .unwrap()
            .matched
    );
    assert_eq!(e.0.mutations.get(), 0);
}

#[tokio::test]
async fn defunct_objects_and_long_text_cannot_prove_a_matching_prefix() {
    let e = fixture();
    e.0.nodes.borrow_mut()[4]
        .element
        .state
        .insert(State::Defunct);
    assert!(
        !e.verify(
            &id(4),
            &Postcondition::State {
                state: State::Checked,
                present: false
            }
        )
        .await
        .unwrap()
        .matched
    );
    e.0.nodes.borrow_mut()[4].element.state = StateSet::empty();
    e.0.nodes.borrow_mut()[4].text.value = "界".repeat(MAX_TEXT_BYTES);
    let result = e
        .verify(
            &id(4),
            &Postcondition::Text("界".repeat(MAX_TEXT_BYTES / 3)),
        )
        .await
        .unwrap();
    assert!(!result.matched);
    let text = result.text.unwrap();
    assert!(text.value.len() <= MAX_TEXT_BYTES);
    assert!(text.truncated);
}
