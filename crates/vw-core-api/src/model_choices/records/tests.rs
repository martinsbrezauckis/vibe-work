//! Storage acceptance tests with real filesystem artifacts at each interrupted replacement boundary.
//! Called by cargo test. No fault hooks or environment switches are present in the shipped writer.
use super::*;
use serde_json::json;
use std::io::Write;

fn scratch() -> tempfile::TempDir {
    let root = std::env::var_os("VW_TEST_ROOT").map_or_else(std::env::temp_dir, PathBuf::from);
    std::fs::create_dir_all(&root).unwrap();
    tempfile::tempdir_in(root).unwrap()
}

fn choice(alias: &str, effort: &str) -> Choice {
    Choice {
        alias: alias.into(),
        effort: effort.into(),
        name: format!("served-{alias}"),
        display_name: format!("Display {alias}"),
    }
}

fn legacy(dir: &Path) -> Vec<Vec<u8>> {
    LEGACY
        .iter()
        .enumerate()
        .map(|(index, (file, _))| {
            let effort = ["off", "high", "provider-future"][index];
            let bytes = serde_json::to_vec(&json!({
                "chat": choice(&format!("gone-{index}"), effort)
            }))
            .unwrap();
            std::fs::write(dir.join(file), &bytes).unwrap();
            bytes
        })
        .collect()
}

fn assert_legacy(dir: &Path, bytes: &[Vec<u8>]) {
    for ((file, _), expected) in LEGACY.iter().zip(bytes) {
        assert_eq!(&std::fs::read(dir.join(file)).unwrap(), expected);
    }
}

#[derive(Clone, Copy, Debug)]
enum Stop {
    BeforeCreate,
    AfterCreate,
    PartialWrite,
    AfterWrite,
    AfterFileSync,
    AfterRename,
    AfterDirectorySync,
}
const STOPS: [Stop; 7] = [
    Stop::BeforeCreate,
    Stop::AfterCreate,
    Stop::PartialWrite,
    Stop::AfterWrite,
    Stop::AfterFileSync,
    Stop::AfterRename,
    Stop::AfterDirectorySync,
];

// The transaction's injectable writer models the disk states of the existing private replacement
// primitive. Before rename, an orphan temp may exist; after rename, exactly the new document exists.
// These tests model process interruption, not physical power loss or filesystem/controller guarantees.
fn interrupt(file: &Path, state: &Document, stop: Stop) -> io::Result<()> {
    let error = || io::Error::other(format!("interrupted at {stop:?}"));
    if matches!(stop, Stop::BeforeCreate) {
        return Err(error());
    }
    let temp = file.with_file_name("model-choices.json.new-interrupted");
    let mut out = vw_platform::create_private_new(&temp).map_err(io::Error::other)?;
    if matches!(stop, Stop::AfterCreate) {
        return Err(error());
    }
    let bytes = serde_json::to_vec(state)?;
    out.write_all(&bytes[..bytes.len() / 2])?;
    if matches!(stop, Stop::PartialWrite) {
        return Err(error());
    }
    out.write_all(&bytes[bytes.len() / 2..])?;
    if matches!(stop, Stop::AfterWrite) {
        return Err(error());
    }
    out.sync_all()?;
    drop(out);
    if matches!(stop, Stop::AfterFileSync) {
        return Err(error());
    }
    if matches!(stop, Stop::AfterRename) {
        std::fs::rename(&temp, file)?;
    } else {
        save(file, state)?;
    }
    Err(error())
}

#[test]
fn migration_retries_every_interrupted_step_without_touching_released_files() {
    for stop in STOPS {
        let dir = scratch();
        let original = legacy(dir.path());
        let records =
            Records::load_with(Some(dir.path()), |file, state| interrupt(file, state, stop));
        assert_eq!(
            records.get("chat").pending.unwrap().effort,
            "off",
            "{stop:?}"
        );
        let restarted = Records::load(Some(dir.path()));
        let record = restarted.get("chat");
        assert_eq!(record.pending.unwrap().alias, "gone-0", "{stop:?}");
        assert_eq!(record.remembered.unwrap().effort, "high", "{stop:?}");
        assert_eq!(
            record.fallback.unwrap().effort,
            "provider-future",
            "{stop:?}"
        );
        assert_legacy(dir.path(), &original);
        assert!(vw_platform::is_user_only(&dir.path().join(FILE)).unwrap());
    }
}

#[test]
fn interrupted_updates_restart_with_the_entire_old_or_entire_new_record() {
    for stop in STOPS {
        let dir = scratch();
        legacy(dir.path());
        let records = Records::load(Some(dir.path()));
        let old = records.get("chat");
        let mut new = Record::default();
        new.accepted(choice("new", "max"));
        let result = records.transaction(
            |state| {
                state
                    .chats
                    .insert("chat".into(), serde_json::to_value(&new)?);
                Ok(true)
            },
            |file, state| interrupt(file, state, stop),
        );
        assert!(result.is_err(), "{stop:?}");
        assert!(
            records.get("chat") == old,
            "failed commit changed live cache: {stop:?}"
        );
        let restarted = Records::load(Some(dir.path()));
        let expected = if matches!(stop, Stop::AfterRename | Stop::AfterDirectorySync) {
            &new
        } else {
            &old
        };
        assert!(
            restarted.get("chat") == *expected,
            "partial record: {stop:?}"
        );
        // Even a commit that renamed successfully but reported a sync error is reconciled before
        // the next edit. Updating another chat must not restore an old cached value over it.
        records
            .update("other", |record| record.accepted(choice("other", "high")))
            .unwrap();
        assert!(
            Records::load(Some(dir.path())).get("chat") == *expected,
            "stale cache: {stop:?}"
        );
    }
}

#[test]
fn many_records_over_the_former_cap_migrate_and_remain_after_another_write() {
    let dir = scratch();
    let mut entries = BTreeMap::new();
    for index in 0..6000 {
        let mut saved = choice(&format!("missing-{index}"), "provider-future");
        saved.display_name = "Long model label ".repeat(12);
        entries.insert(format!("chat-{index}"), saved);
    }
    let bytes = serde_json::to_vec(&entries).unwrap();
    assert!(bytes.len() > 1024 * 1024);
    for (file, _) in LEGACY {
        std::fs::write(dir.path().join(file), &bytes).unwrap();
    }
    let records = Records::load(Some(dir.path()));
    records
        .update("new", |record| record.accepted(choice("new", "off")))
        .unwrap();
    let restarted = Records::load(Some(dir.path()));
    for (id, expected) in entries {
        let actual = restarted.get(&id);
        assert!(actual.pending.as_ref() == Some(&expected), "{id}");
        assert!(actual.remembered.as_ref() == Some(&expected), "{id}");
        assert!(actual.fallback.as_ref() == Some(&expected), "{id}");
    }
}

#[test]
fn newer_choices_and_deletions_survive_second_start_and_rollback_snapshots() {
    let dir = scratch();
    let original = legacy(dir.path());
    let records = Records::load(Some(dir.path()));
    records
        .update("chat", |record| record.accepted(choice("new-pick", "off")))
        .unwrap();
    records
        .update("new-chat", |record| {
            record.accepted(choice("new-chat", "max"));
        })
        .unwrap();
    // The released reader can still consume all three originals during a downgrade. That older
    // app cannot display post-upgrade picks, but it cannot erase them from the new store either.
    assert_legacy(dir.path(), &original);
    let restarted = Records::load(Some(dir.path()));
    assert_eq!(restarted.get("chat").pending.unwrap().alias, "new-pick");
    assert_eq!(restarted.get("new-chat").pending.unwrap().effort, "max");
    restarted.forget("chat");
    let again = Records::load(Some(dir.path()));
    assert!(
        again.get("chat").expected().is_none(),
        "legacy snapshot resurrected a deleted choice"
    );
    assert!(again.get("new-chat").expected().is_some());
    assert_legacy(dir.path(), &original);
}

#[test]
fn damaged_rows_keep_other_chats_and_are_retained_until_explicitly_replaced() {
    let dir = scratch();
    let values = json!({
        "known": {"alias":"large", "effort":"high"},
        "future": {"alias":"custom", "effort":"provider-future"},
        "long": {"alias":"a".repeat(129), "effort":"p".repeat(129)},
        "damaged": {"alias":"custom", "effort":17},
        "blank": {"alias":"custom", "effort":" \t"},
        "spaced": {"alias":"custom", "effort":"two words"}
    });
    std::fs::write(
        dir.path().join(LEGACY[0].0),
        serde_json::to_vec(&values).unwrap(),
    )
    .unwrap();
    let records = Records::load(Some(dir.path()));
    assert_eq!(records.get("known").pending.unwrap().effort, "high");
    assert_eq!(
        records.get("future").pending.unwrap().effort,
        "provider-future"
    );
    assert!(records.get("long").pending.is_some());
    for id in ["damaged", "blank", "spaced"] {
        assert!(records.get(id).expected().is_none());
    }
    records
        .update("new", |record| record.accepted(choice("new", "off")))
        .unwrap();
    let (disk, _) = read(dir.path()).unwrap();
    for id in ["damaged", "blank", "spaced"] {
        assert_eq!(disk.chats[id]["pending"], values[id]);
    }
}

#[test]
fn migration_preserves_all_combinations_including_an_interrupted_old_acceptance() {
    let dir = scratch();
    for (slot, (file, _)) in LEGACY.iter().enumerate() {
        let mut values = serde_json::Map::new();
        for mask in 1..8 {
            if mask & (1 << slot) != 0 {
                values.insert(
                    format!("chat-{mask}"),
                    serde_json::to_value(choice(
                        &format!("alias-{slot}"),
                        ["off", "high", "provider-future"][slot],
                    ))
                    .unwrap(),
                );
            }
        }
        std::fs::write(dir.path().join(file), serde_json::to_vec(&values).unwrap()).unwrap();
    }
    let records = Records::load(Some(dir.path()));
    let (document, _) = read(dir.path()).unwrap();
    for mask in 1..8 {
        let id = format!("chat-{mask}");
        for (slot, (_, field)) in LEGACY.iter().enumerate() {
            assert_eq!(
                document.chats[&id].get(field).is_some(),
                mask & (1 << slot) != 0
            );
        }
        let expected = if mask & 4 != 0 {
            "alias-2"
        } else if mask & 1 != 0 {
            "alias-0"
        } else {
            "alias-1"
        };
        assert_eq!(records.get(&id).expected().unwrap().alias, expected);
    }
    // A new accepted pick clears an older fallback and updates the remembered identity together.
    records
        .update("chat-7", |record| record.accepted(choice("fresh", "off")))
        .unwrap();
    let record = Records::load(Some(dir.path())).get("chat-7");
    assert!(record.fallback.is_none());
    assert!(record.pending == record.remembered);
    assert_eq!(record.pending.unwrap().effort, "off");
}

#[test]
fn refused_writes_keep_all_previous_choice_fields_and_can_be_retried() {
    let dir = scratch();
    legacy(dir.path());
    let records = Records::load(Some(dir.path()));
    let before = records.get("chat");
    let file = dir.path().join(FILE);
    let bytes = std::fs::read(&file).unwrap();
    std::fs::remove_file(&file).unwrap();
    std::fs::create_dir(&file).unwrap();
    assert!(
        records
            .update("chat", |record| record.replaced(choice("default", "off")))
            .is_err()
    );
    assert!(records.get("chat") == before);
    std::fs::remove_dir(&file).unwrap();
    std::fs::write(&file, bytes).unwrap();
    records
        .update("chat", |record| record.replaced(choice("default", "off")))
        .unwrap();
    let record = Records::load(Some(dir.path())).get("chat");
    assert!(record.pending.is_none());
    assert!(record.fallback == record.remembered);
    assert_eq!(record.fallback.unwrap().alias, "default");
}

#[test]
fn unreadable_or_future_store_is_never_overwritten_with_an_empty_map() {
    for bytes in [
        b"{broken".as_slice(),
        br#"{"version":99,"chats":{}}"#.as_slice(),
    ] {
        let dir = scratch();
        let original = legacy(dir.path());
        std::fs::write(dir.path().join(FILE), bytes).unwrap();
        let records = Records::load(Some(dir.path()));
        assert!(
            records
                .update("new", |record| record.accepted(choice("new", "off")))
                .is_err()
        );
        assert_eq!(std::fs::read(dir.path().join(FILE)).unwrap(), bytes);
        assert_legacy(dir.path(), &original);
    }
}

#[test]
fn unreadable_legacy_file_prevents_partial_migration_and_recovers_after_repair() {
    let dir = scratch();
    let original = legacy(dir.path());
    std::fs::write(dir.path().join(LEGACY[1].0), b"{broken").unwrap();
    let records = Records::load(Some(dir.path()));
    assert!(!dir.path().join(FILE).exists());
    assert!(
        records
            .update("new", |record| record.accepted(choice("new", "off")))
            .is_err()
    );
    std::fs::write(dir.path().join(LEGACY[1].0), &original[1]).unwrap();
    assert_eq!(
        Records::load(Some(dir.path()))
            .get("chat")
            .remembered
            .unwrap()
            .effort,
        "high"
    );
    assert_legacy(dir.path(), &original);
}

#[test]
fn concurrent_windows_merge_chats_under_the_commit_lock() {
    let dir = scratch();
    let left = Records::load(Some(dir.path()));
    let right = Records::load(Some(dir.path()));
    std::thread::scope(|scope| {
        for (prefix, records) in [("left", &left), ("right", &right)] {
            scope.spawn(move || {
                for index in 0..20 {
                    let id = format!("{prefix}-{index}");
                    records
                        .update(&id, |record| record.accepted(choice(&id, "high")))
                        .unwrap();
                }
            });
        }
    });
    let restarted = Records::load(Some(dir.path()));
    for prefix in ["left", "right"] {
        for index in 0..20 {
            assert!(
                restarted
                    .get(&format!("{prefix}-{index}"))
                    .pending
                    .is_some()
            );
        }
    }
}

#[test]
fn fallback_retirement_and_handoff_keep_the_whole_choice() {
    let dir = scratch();
    let records = Records::load(Some(dir.path()));
    records
        .update("old", |record| record.accepted(choice("missing", "high")))
        .unwrap();
    records
        .update("old", |record| record.replaced(choice("default", "off")))
        .unwrap();
    records.copy("old", "new").unwrap();
    let restarted = Records::load(Some(dir.path()));
    for id in ["old", "new"] {
        let record = restarted.get(id);
        assert!(record.pending.is_none());
        assert!(record.fallback == record.remembered);
        assert_eq!(record.fallback.unwrap().effort, "off");
    }
    restarted
        .update("new", |record| {
            record.pending = None;
            record.fallback = None;
        })
        .unwrap();
    let restarted = Records::load(Some(dir.path()));
    assert!(restarted.get("new").replay().is_none());
    assert_eq!(
        restarted.get("new").remembered.unwrap().name,
        "served-default"
    );
    assert!(restarted.get("old").fallback.is_some());
}

#[test]
fn recovery_keeps_readable_choices_when_the_commit_lock_is_unavailable() {
    let dir = scratch();
    let records = Records::load(Some(dir.path()));
    records
        .update("chat", |record| record.accepted(choice("chosen", "high")))
        .unwrap();
    let lock_path = dir.path().join("model-choices.lock");
    std::fs::remove_file(&lock_path).unwrap();
    std::fs::create_dir(&lock_path).unwrap();
    let restarted = Records::load(Some(dir.path()));
    assert!(
        restarted.get("chat").pending.is_some(),
        "a write-lock error hid a readable saved choice"
    );
    assert!(
        restarted
            .update("chat", |record| record.accepted(choice("new", "off")))
            .is_err()
    );
}

#[test]
fn recovery_keeps_valid_legacy_slots_when_another_file_is_broken() {
    let dir = scratch();
    legacy(dir.path());
    std::fs::write(dir.path().join("chat-models.json"), b"{broken").unwrap();
    let records = Records::load(Some(dir.path()));
    let record = records.get("chat");
    assert!(
        record.pending.is_some(),
        "a damaged remembered map hid the pending choices"
    );
    assert!(
        record.fallback.is_some(),
        "a damaged remembered map hid the fallbacks"
    );
    assert!(
        !dir.path().join(FILE).exists(),
        "a partial migration was committed"
    );
    assert!(
        records
            .update("new", |record| record.accepted(choice("new", "off")))
            .is_err()
    );
}
