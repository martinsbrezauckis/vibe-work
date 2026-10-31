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
