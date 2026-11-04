//! `State::retire_auto_project`: only an empty automatic project for exactly that folder goes.
use super::*;
use crate::{NewSession, Seen};

fn seen(id: &str, folder: &str) -> Seen {
    Seen {
        id: id.to_owned(),
        folder: Some(folder.to_owned()),
    }
}

#[test]
fn only_an_empty_automatic_project_for_the_folder_is_retired() {
    let mut state = State::default();
    // An automatic project whose only chat was forgotten (a helper chat), one with a chat, one a person made.
    state.file_seen(
        &[seen("helper", "/data/work"), seen("kept", "/code/app")],
        1,
    );
    assert!(state.forget("helper"));
    let made = state.create("Work notes", Some("/data/work/"), 2).unwrap();
    assert_eq!(state.projects().len(), 3);
    assert!(state.retire_auto_project("/data/work"));
    let left: Vec<&str> = state.projects().iter().map(|p| p.name.as_str()).collect();
    assert_eq!(left.len(), 2, "{left:?}");
    assert!(
        state.project(&made.id).is_some(),
        "a person's project stays"
    );
    assert!(
        state
            .projects()
            .iter()
            .any(|p| p.main_folder.as_deref() == Some("/code/app")),
        "another folder's project stays"
    );
    // Once gone, nothing more to do; an automatic project that has a chat is never removed.
    assert!(!state.retire_auto_project("/data/work"));
    assert!(!state.retire_auto_project("/code/app"));
    let filed = state.file_new(
        &NewSession {
            id: "chat",
            hint: None,
            source: None,
            folder: Some("/other"),
        },
        3,
    );
    assert!(filed);
    assert!(!state.retire_auto_project("/other"));
}

/// `State::name_auto_project`: the automatic project of a folder Vibe Work made (a Temporary chat's own folder)
/// shows the given name instead of the folder's random name; a second one gets a number; a person's name stays.
#[test]
fn an_automatic_project_takes_the_given_name_once() {
    let mut state = State::default();
    let a = "/data/sessions/temporary-folders/chat-00aa11bb22cc33dd";
    let b = "/data/sessions/temporary-folders/chat-44ee55ff66007788";
    state.file_seen(&[seen("a", a), seen("b", b)], 1);
    assert!(state.name_auto_project(a, "Temporary chat"));
    assert!(state.name_auto_project(b, "Temporary chat"));
    let names: Vec<&str> = state.projects().iter().map(|p| p.name.as_str()).collect();
    assert!(names.contains(&"Temporary chat"), "{names:?}");
    assert!(names.contains(&"Temporary chat (2)"), "{names:?}");
    assert!(
        !state.name_auto_project(a, "Temporary chat"),
        "already named"
    );
    let id = state
        .projects()
        .iter()
        .find(|p| p.main_folder.as_deref() == Some(b))
        .unwrap()
        .id
        .clone();
    state.rename(&id, "Kept").unwrap();
    assert!(
        !state.name_auto_project(b, "Temporary chat"),
        "a person's name stays"
    );
    assert!(!state.name_auto_project("/elsewhere", "Temporary chat"));
}
