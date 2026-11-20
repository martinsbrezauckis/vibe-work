//! Knowledge base store, validation and agent text.
//!
//! PREDICT (written before the module existed): every test here fails to build on the base, which has no
//! knowledge base at all (no `KnowledgeRequest`, no `knowledgePlaces` setting, no agent text); once the store
//! exists, a folder that is missing / a file / inside the vault or app data, a name or note that looks like a
//! key, a duplicate name or place, and a hand-edited settings file with bad entries must each be refused or
//! dropped, and the agent text must list every place with the "pointer only" rule.

use std::path::Path;

use serde_json::{Value, json};

use super::{
    FolderRules, MAX_NOTE, MAX_PLACES, SETTINGS_KEY, agent_text, apply, load, looks_secret, run,
};
use crate::knowledge_view::{KnowledgeKind, KnowledgePlace, KnowledgeRequest};

/// No folder is excluded, except under `blocked` when given.
struct Rules(Option<std::path::PathBuf>);

impl FolderRules for Rules {
    fn excluded(&self, folder: &Path) -> bool {
        self.0.as_ref().is_some_and(|b| folder.starts_with(b))
    }
}

fn open() -> Rules {
    Rules(None)
}

fn add(name: &str, kind: KnowledgeKind, location: &str) -> KnowledgeRequest {
    KnowledgeRequest::Add {
        name: name.to_owned(),
        kind,
        location: location.to_owned(),
        label: None,
        note: None,
    }
}

fn folder_place(name: &str, dir: &Path) -> Vec<KnowledgePlace> {
    apply(
        &[],
        add(name, KnowledgeKind::Folder, dir.to_str().unwrap()),
        &open(),
    )
    .unwrap()
}

#[test]
fn a_folder_place_is_added_with_a_fresh_id_and_the_path_as_given() {
    let dir = tempfile::tempdir().unwrap();
    let places = folder_place("  Our library ", dir.path());
    assert_eq!(places.len(), 1);
    assert_eq!(places[0].name, "Our library");
    assert_eq!(places[0].kind, KnowledgeKind::Folder);
    assert_eq!(places[0].location, dir.path().to_str().unwrap());
    assert!(places[0].id.starts_with("kb-") && places[0].id.len() == 19);
}

#[test]
fn a_missing_folder_a_file_and_a_relative_path_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("notes.txt");
    std::fs::write(&file, "x").unwrap();
    let missing = dir.path().join("gone");
    for (location, words) in [
        (missing.to_str().unwrap(), "No folder at this path"),
        (file.to_str().unwrap(), "not a folder"),
        ("relative/folder", "full path"),
    ] {
        let error = apply(&[], add("Lib", KnowledgeKind::Folder, location), &open()).unwrap_err();
        assert!(error.contains(words), "{location}: {error}");
    }
}

#[test]
fn a_folder_inside_the_vault_or_app_data_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let inner = dir.path().join("vault");
    std::fs::create_dir(&inner).unwrap();
    let rules = Rules(Some(dir.path().canonicalize().unwrap()));
    let error = apply(
        &[],
        add("Locked", KnowledgeKind::Folder, inner.to_str().unwrap()),
        &rules,
    )
    .unwrap_err();
    assert!(error.contains("never reads"), "{error}");
}

#[test]
fn names_and_notes_that_look_like_secrets_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_str().unwrap();
    // Fake, credential-shaped values only.
    for name in [
        concat!("gh", "p_abcdefghijklmnopqrstuvwxyz0123"),
        "password=hunter2hunter2",
        concat!("sk", "-test0000000000000000"),
    ] {
        let error = apply(&[], add(name, KnowledgeKind::Folder, path), &open()).unwrap_err();
        assert!(error.contains("looks like a password or key"), "{error}");
    }
    let error = apply(
        &[],
        KnowledgeRequest::Add {
            name: "Lib".into(),
            kind: KnowledgeKind::Folder,
            location: path.into(),
            label: None,
            note: Some("token: abcdefabcdefabcdef".into()),
        },
        &open(),
    )
    .unwrap_err();
    assert!(error.contains("The note looks like"), "{error}");
    assert!(!looks_secret("Contracts and offers since 2024"));
}

#[test]
fn empty_long_and_multi_line_names_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_str().unwrap();
    let long = "x".repeat(61);
    for (name, words) in [
        ("   ", "empty"),
        (long.as_str(), "longer than 60"),
        ("two\nlines", "one line"),
    ] {
        let error = apply(&[], add(name, KnowledgeKind::Folder, path), &open()).unwrap_err();
        assert!(error.contains(words), "{name:?}: {error}");
    }
}

#[test]
fn a_duplicate_name_or_place_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_str().unwrap();
    let places = folder_place("Library", dir.path());
    let other = tempfile::tempdir().unwrap();
    let same_name = apply(
        &places,
        add(
            "library",
            KnowledgeKind::Folder,
            other.path().to_str().unwrap(),
        ),
        &open(),
    )
    .unwrap_err();
    assert!(same_name.contains("already in the list"), "{same_name}");
    let same_place = apply(&places, add("Docs", KnowledgeKind::Folder, path), &open()).unwrap_err();
    assert!(same_place.contains("as \"Library\""), "{same_place}");
}

#[test]
fn drives_and_mistral_libraries_are_added_without_any_call() {
    let drive = apply(
        &[],
        add("Team drive", KnowledgeKind::Drive, "Google Drive"),
        &open(),
    )
    .unwrap();
    let both = apply(
        &drive,
        KnowledgeRequest::Add {
            name: "Research".into(),
            kind: KnowledgeKind::MistralLibrary,
            location: "0198a-lib".into(),
            label: Some("Research papers".into()),
            note: None,
        },
        &open(),
    )
    .unwrap();
    assert_eq!(both.len(), 2);
    assert_eq!(both[1].label.as_deref(), Some("Research papers"));
    let bad_id = apply(
        &[],
        add("X", KnowledgeKind::MistralLibrary, "../../etc"),
        &open(),
    )
    .unwrap_err();
    assert!(bad_id.contains("Mistral Libraries"), "{bad_id}");
}

#[test]
fn rename_and_remove_change_only_the_named_place() {
    let dir = tempfile::tempdir().unwrap();
    let places = apply(
        &folder_place("Library", dir.path()),
        add("Drive", KnowledgeKind::Drive, "OneDrive"),
        &open(),
    )
    .unwrap();
    let id = places[0].id.clone();
    let renamed = apply(
        &places,
        KnowledgeRequest::Rename {
            id: id.clone(),
            name: "Our library".into(),
        },
        &open(),
    )
    .unwrap();
    assert_eq!(renamed[0].name, "Our library");
    assert_eq!(renamed[1], places[1]);
    // Renaming to its own name in other letter case is not a duplicate.
    apply(
        &renamed,
        KnowledgeRequest::Rename {
            id: id.clone(),
            name: "OUR LIBRARY".into(),
        },
        &open(),
    )
    .unwrap();
    let clash = apply(
        &renamed,
        KnowledgeRequest::Rename {
            id: id.clone(),
            name: "drive".into(),
        },
        &open(),
    )
    .unwrap_err();
    assert!(clash.contains("already in the list"), "{clash}");
    let removed = apply(
        &renamed,
        KnowledgeRequest::Remove { id: id.clone() },
        &open(),
    )
    .unwrap();
    assert_eq!(removed, vec![places[1].clone()]);
    let gone = apply(&removed, KnowledgeRequest::Remove { id }, &open()).unwrap_err();
    assert!(gone.contains("no longer in the list"), "{gone}");
}

// PREDICT (red before the change): there is no `setNote` request, so a place's note could only be changed by
// removing and adding the place again (a new id); this does not compile until the request exists.
#[test]
fn set_note_changes_or_clears_only_the_note_of_the_named_place() {
    let dir = tempfile::tempdir().unwrap();
    let places = apply(
        &folder_place("Library", dir.path()),
        add("Drive", KnowledgeKind::Drive, "OneDrive"),
        &open(),
    )
    .unwrap();
    let id = places[0].id.clone();
    let noted = apply(
        &places,
        KnowledgeRequest::SetNote {
            id: id.clone(),
            note: "  contracts since 2024 ".into(),
        },
        &open(),
    )
    .unwrap();
    assert_eq!(noted[0].note.as_deref(), Some("contracts since 2024"));
    assert_eq!(noted[0].id, id);
    assert_eq!(noted[0].name, places[0].name);
    assert_eq!(noted[1], places[1]);
    // An empty note removes it.
    let cleared = apply(
        &noted,
        KnowledgeRequest::SetNote {
            id: id.clone(),
            note: "   ".into(),
        },
        &open(),
    )
    .unwrap();
    assert_eq!(cleared[0].note, None);
    let secret = apply(
        &noted,
        KnowledgeRequest::SetNote {
            id: id.clone(),
            note: "password=hunter2hunter2".into(),
        },
        &open(),
    )
    .unwrap_err();
    assert!(secret.contains("looks like a password"), "{secret}");
    let long = apply(
        &noted,
        KnowledgeRequest::SetNote {
            id: id.clone(),
            note: "x".repeat(MAX_NOTE + 1),
        },
        &open(),
    )
    .unwrap_err();
    assert!(long.contains("longer than"), "{long}");
    let gone = apply(
        &noted,
        KnowledgeRequest::SetNote {
            id: "kb-0000".into(),
            note: "x".into(),
        },
        &open(),
    )
    .unwrap_err();
    assert!(gone.contains("no longer in the list"), "{gone}");
}

#[test]
fn the_list_is_capped() {
    let full: Vec<KnowledgePlace> = (0..MAX_PLACES)
        .map(|i| KnowledgePlace {
            id: format!("kb-{i}"),
            name: format!("Place {i}"),
            kind: KnowledgeKind::Drive,
            location: format!("Drive {i}"),
            label: None,
            note: None,
        })
        .collect();
    let error = apply(
        &full,
        add("One more", KnowledgeKind::Drive, "Dropbox"),
        &open(),
    )
    .unwrap_err();
    assert!(error.contains("at most 50"), "{error}");
}

#[test]
fn run_saves_into_settings_and_keeps_other_settings() {
    let data = tempfile::tempdir().unwrap();
    let settings = data.path().join("settings.json");
    std::fs::write(&settings, r#"{"keepAwake":false}"#).unwrap();
    let folder = tempfile::tempdir().unwrap();
    let places = run(
        &settings,
        add(
            "Library",
            KnowledgeKind::Folder,
            folder.path().to_str().unwrap(),
        ),
        &open(),
    )
    .unwrap();
    assert!(places.1, "an add changes the list");
    let places = places.0;
    let saved: Value = serde_json::from_slice(&std::fs::read(&settings).unwrap()).unwrap();
    assert_eq!(saved["keepAwake"], false);
    assert_eq!(saved[SETTINGS_KEY][0]["name"], "Library");
    assert_eq!(
        run(&settings, KnowledgeRequest::List {}, &open()).unwrap(),
        (places.clone(), false),
        "a list changes nothing"
    );
    assert_eq!(load(&settings), places);
    // A refused request leaves the file as it was.
    let before = std::fs::read(&settings).unwrap();
    assert!(
        run(
            &settings,
            add("Library", KnowledgeKind::Drive, "x"),
            &open()
        )
        .is_err()
    );
    assert_eq!(std::fs::read(&settings).unwrap(), before);
    // Removing the last place removes the key.
    run(
        &settings,
        KnowledgeRequest::Remove {
            id: places[0].id.clone(),
        },
        &open(),
    )
    .unwrap();
    let saved: Value = serde_json::from_slice(&std::fs::read(&settings).unwrap()).unwrap();
    assert!(saved.get(SETTINGS_KEY).is_none());
    assert_eq!(saved["keepAwake"], false);
}

#[test]
fn a_hand_edited_file_never_reaches_the_agent_unchecked() {
    let data = tempfile::tempdir().unwrap();
    let settings = data.path().join("settings.json");
    let good = json!({"id":"kb-1","name":"Library","kind":"folder","location":"/srv/library"});
    std::fs::write(
        &settings,
        json!({ SETTINGS_KEY: [
            good,
            {"id":"kb-2","name":concat!("gh", "p_abcdefghijklmnopqrstuvwxyz0123"),"kind":"drive","location":"x"},
            {"id":"kb-3","name":"library","kind":"drive","location":"dup name"},
            {"id":"kb-4","name":"Two\nlines","kind":"drive","location":"x"},
            {"id":"kb-1","name":"Same id","kind":"drive","location":"x"},
            {"id":"kb-5","name":"Bad lib","kind":"mistralLibrary","location":"a/b"},
            {"id":"kb-6","name":"Extra","kind":"drive","location":"x","unknown":1},
            "not a place"
        ]})
        .to_string(),
    )
    .unwrap();
    let places = load(&settings);
    assert_eq!(places.len(), 1, "{places:?}");
    assert_eq!(places[0].name, "Library");
    std::fs::write(&settings, "not json").unwrap();
    assert_eq!(load(&settings), Vec::new());
    assert_eq!(load(&data.path().join("missing.json")), Vec::new());
}

#[test]
fn the_agent_text_lists_every_place_with_the_pointer_rule() {
    assert_eq!(agent_text(&[]), None);
    let places = vec![
        KnowledgePlace {
            id: "kb-1".into(),
            name: "Our library".into(),
            kind: KnowledgeKind::Folder,
            location: "/srv/library".into(),
            label: None,
            note: Some("contracts since 2024".into()),
        },
        KnowledgePlace {
            id: "kb-2".into(),
            name: "Team drive".into(),
            kind: KnowledgeKind::Drive,
            location: "Google Drive".into(),
            label: None,
            note: None,
        },
        KnowledgePlace {
            id: "kb-3".into(),
            name: "Research \"best\"".into(),
            kind: KnowledgeKind::MistralLibrary,
            location: "0198a-lib".into(),
            label: Some("Research papers".into()),
            note: None,
        },
    ];
    let text = agent_text(&places).unwrap();
    for words in [
        "look in that place first instead of searching the whole computer",
        "gives no extra access",
        "never copy its files elsewhere on your own",
        "only when the user asks for it",
        "- \"Our library\": folder \"/srv/library\". Note: \"contracts since 2024\"",
        "- \"Team drive\": cloud drive, not on this computer; reach it through the user's connector \"Google Drive\"",
        "- \"Research \\\"best\\\"\": Mistral Library \"Research papers\" (id 0198a-lib); search it with library_search",
    ] {
        assert!(text.contains(words), "missing {words:?} in:\n{text}");
    }
    // One line per place after the rule: user text never adds lines.
    assert_eq!(text.lines().count(), 4);
}

// PREDICT: passes before and after: the core takes a Windows path's backslashes as typed (`\t` in `\tester` is two
// characters, never a tab). A "must be one line of text" refusal for such a path means a real tab character reached
// the field, which only a typing tool that turns `\t` into Tab produces.
#[test]
fn a_windows_path_with_backslashes_is_taken_as_typed() {
    let typed = r"C:\Users\tester\AppData\Roaming\notes";
    let error = apply(&[], add("Notes", KnowledgeKind::Folder, typed), &open()).unwrap_err();
    assert!(!error.contains("one line"), "{error}");
    #[cfg(windows)]
    {
        let dir = tempfile::tempdir().unwrap();
        let inner = dir.path().join("tester");
        std::fs::create_dir(&inner).unwrap();
        let typed = inner.to_str().unwrap();
        assert!(typed.contains(r"\tester"), "{typed}");
        let places = apply(&[], add("Tester", KnowledgeKind::Folder, typed), &open()).unwrap();
        assert_eq!(places[0].location, typed);
    }
}
