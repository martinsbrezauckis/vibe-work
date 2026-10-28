//! Scanner policy and incremental falsifiers; no runtime Vault or user log files are read.
//! Depends on scheduling/cache helpers, invented inputs and temporary folders; complements dispatcher integration tests.

use crate::{
    SecretLeakName,
    secret_scan::ready,
    secret_scan_cache::{Cache, CachedFile, Stamp, fingerprint},
    secret_scan_worker::filter_inputs,
};
use std::{collections::BTreeMap, path::PathBuf, time::Duration};
use vw_keys::VaultScanSecret;
use vw_vault::SecretString;

fn input(id: &str, modified_ms: i64, value: &str) -> VaultScanSecret {
    VaultScanSecret {
        id: id.to_owned(),
        name: id.to_owned(),
        login_id: None,
        modified_ms,
        website_password: false,
        value: SecretString::exact(value),
    }
}

#[test]
fn strength_filter_excludes_guessable_values_and_only_exports_a_count() {
    let weak = [
        "1234567",
        "password1",
        "P@ssword1",
        "remember milk",
        &"x".repeat(80),
    ];
    let mut secrets: Vec<_> = weak.iter().map(|value| input(value, 1, value)).collect();
    secrets.extend([
        input("password", 1, "Q7!mV2@rL9#xT4$kN8%pZ6&s"),
        input("token", 1, "sk-X4u2B9f6W1s8J5n0R7k3T6q9M2v4H8z1"),
    ]);
    let (filtered, skipped) = filter_inputs(secrets);
    assert_eq!(skipped, 5);
    assert_eq!(filtered.len(), 2);
    assert_eq!(filtered[0].id, "password");
    assert_eq!(filtered[1].id, "token");
    let view = crate::SecretScanView {
        skipped_secrets: skipped,
        ..crate::SecretScanView::default()
    };
    let serialized = serde_json::to_value(view).unwrap();
    assert_eq!(serialized["skippedSecrets"], 5);
    assert!(serialized.get("guessesLog10").is_none());
    assert!(serialized.get("skippedValues").is_none());
}

#[test]
fn strength_filter_uses_sixteen_log_guesses_instead_of_the_maximum_score() {
    let below = "f9K2m7Q4v8R1z5T";
    let enough = "f9K2m7Q4v8R1z5T$";
    assert_eq!(zxcvbn::zxcvbn(below, &[]).score(), zxcvbn::Score::Four);
    assert!(zxcvbn::zxcvbn(below, &[]).guesses_log10() < 16.0);
    assert!(zxcvbn::zxcvbn(enough, &[]).guesses_log10() >= 16.0);
    let (filtered, skipped) =
        filter_inputs(vec![input("below", 1, below), input("enough", 1, enough)]);
    assert_eq!(skipped, 1);
    assert_eq!(filtered[0].id, "enough");
}

#[test]
fn startup_requires_loaded_list_and_thirty_idle_seconds_and_gives_way_to_turns() {
    assert!(!ready(false, Duration::from_secs(100), false, false));
    assert!(!ready(true, Duration::from_secs(29), false, false));
    assert!(ready(true, Duration::from_secs(30), false, false));
    assert!(!ready(true, Duration::from_secs(100), true, false));
    assert!(!ready(false, Duration::ZERO, false, true));
    assert!(ready(true, Duration::ZERO, false, true));
}

#[test]
fn incremental_uses_size_mtime_and_sorted_ids_and_times_without_hashing_values() {
    let secrets = vec![
        input("a", 10, "invented-value-a"),
        input("b", 20, "invented-value-b"),
    ];
    let fingerprint = fingerprint(&secrets);
    assert_eq!(
        fingerprint,
        super::secret_scan_cache::fingerprint(&[
            input("b", 20, "changed-value"),
            input("a", 10, "other-value")
        ])
    );
    assert_ne!(
        fingerprint,
        super::secret_scan_cache::fingerprint(&[
            input("a", 11, "invented-value-a"),
            input("b", 20, "invented-value-b")
        ])
    );
    let path = PathBuf::from("messages.jsonl");
    let stamp = Stamp {
        size: 100,
        modified_ns: 42,
    };
    let mut cache = Cache::default();
    assert!(cache.changed(&path, &stamp, &fingerprint));
    cache.files.insert(
        path.clone(),
        CachedFile {
            stamp: stamp.clone(),
            fingerprint: fingerprint.clone(),
            chat_id: "chat".to_owned(),
            title: "Fix login".to_owned(),
            hits: BTreeMap::<String, SecretLeakName>::new(),
            places: 0,
        },
    );
    assert!(!cache.changed(&path, &stamp, &fingerprint));
    assert!(cache.changed(&path, &Stamp { size: 101, ..stamp }, &fingerprint));
    assert!(cache.changed(
        &path,
        &Stamp {
            modified_ns: 43,
            ..stamp
        },
        &fingerprint
    ));
    assert!(cache.changed(&path, &stamp, "changed-secret-set"));
    // A cache saved while automatic redaction existed still loads, and its retry flag no longer rechecks a file.
    let mut older = serde_json::to_value(&cache).unwrap();
    older["files"]["messages.jsonl"]["retry_auto"] = serde_json::json!(true);
    let older: Cache = serde_json::from_value(older).unwrap();
    assert!(!older.changed(&path, &stamp, &fingerprint));
    let text = serde_json::to_string(&cache).unwrap();
    assert!(!text.contains("invented-value-a"));
    assert!(!text.contains("invented-value-b"));
}

/// A fresh folder below the temporary folder, as written (not canonicalized: on macOS it lies below `/var`).
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "vw-log-folder-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The session log folder is configured through a symlinked folder and the scanner lists files under it as
/// configured: those files are inside. A file that is itself a link to a file outside, a `..` hop out of the
/// folder, a sibling file and a missing folder stay outside.
#[cfg(unix)]
#[test]
fn log_files_under_a_linked_log_folder_are_inside_and_escapes_stay_outside() {
    use crate::secret_scan_worker::inside_log_folder;
    let base = scratch("linked");
    let real = base.join("real-logs");
    std::fs::create_dir_all(real.join("chat")).unwrap();
    std::fs::write(real.join("chat/messages.jsonl"), b"{}").unwrap();
    std::fs::write(base.join("outside.jsonl"), b"{}").unwrap();
    let linked = base.join("configured-logs");
    std::os::unix::fs::symlink(&real, &linked).unwrap();
    std::os::unix::fs::symlink(base.join("outside.jsonl"), real.join("chat/escape.jsonl")).unwrap();

    assert!(inside_log_folder(&real, &real.join("chat/messages.jsonl")));
    assert!(inside_log_folder(
        &linked,
        &linked.join("chat/messages.jsonl")
    ));
    assert!(inside_log_folder(
        &linked,
        &real.join("chat/messages.jsonl")
    ));
    assert!(inside_log_folder(
        &real,
        &linked.join("chat/messages.jsonl")
    ));

    assert!(!inside_log_folder(
        &linked,
        &linked.join("chat/escape.jsonl")
    ));
    assert!(!inside_log_folder(
        &linked,
        &linked.join("../outside.jsonl")
    ));
    assert!(!inside_log_folder(&linked, &base.join("outside.jsonl")));
    assert!(!inside_log_folder(
        &linked,
        &linked.join("chat/missing.jsonl")
    ));
    assert!(!inside_log_folder(
        &base.join("gone"),
        &linked.join("chat/messages.jsonl")
    ));
    std::fs::remove_dir_all(base).unwrap();
}

/// Every Windows spelling of the configured log folder (as written, verbatim `\\?\C:\...`, plain, `/` separators,
/// upper and lower case) holds the files listed under any other spelling; a sibling file stays outside.
#[cfg(windows)]
#[test]
fn log_files_match_every_windows_spelling_of_the_log_folder() {
    use crate::secret_scan_worker::inside_log_folder;
    let base = scratch("spellings");
    let as_written = base.join("logs");
    std::fs::create_dir_all(as_written.join("chat")).unwrap();
    std::fs::write(as_written.join("chat").join("messages.jsonl"), b"{}").unwrap();
    std::fs::write(base.join("outside.jsonl"), b"{}").unwrap();
    let verbatim = as_written.canonicalize().unwrap();
    assert!(verbatim.to_string_lossy().starts_with(r"\\?\"));
    let plain = vw_platform::plain_path(&verbatim);
    let slashes = PathBuf::from(plain.to_string_lossy().replace('\\', "/"));
    let upper = PathBuf::from(plain.to_string_lossy().to_uppercase());
    let lower = PathBuf::from(plain.to_string_lossy().to_lowercase());
    let spellings = [&as_written, &verbatim, &plain, &slashes, &upper, &lower];
    for folder in spellings {
        for listed in spellings {
            let file = listed.join("chat").join("messages.jsonl");
            assert!(
                inside_log_folder(folder, &file),
                "folder {} file {}",
                folder.display(),
                file.display()
            );
        }
        assert!(!inside_log_folder(folder, &base.join("outside.jsonl")));
    }
    std::fs::remove_dir_all(base).unwrap();
}

/// Which chat a log file belongs to, by layout. Vibe 3.0's unified store names its owner in
/// `unified/<id>/meta.json` for files up to two folders deeper, but only when that `meta.json` names the folder's
/// own id; the 2.x layout keeps `meta.json` beside `messages.jsonl`; a file of no store is no chat's. Every file of
/// a unified store is sealed (never edited in place); 2.x files are not; Vibe 3.0's rebuildable chat index is not
/// checked at all.
#[test]
fn unified_store_files_belong_to_their_store_meta_and_are_sealed_while_2x_files_are_not() {
    use crate::secret_scan_cache::{chat, derived_index, sealed};
    let folder = scratch("unified");
    let id = "8475dd14-a81e-faf3-5d01-9a0b97789dab";
    let store = folder.join("unified").join(id);
    for dir in ["journal", "generations/0000000000000008", "chunks"] {
        std::fs::create_dir_all(store.join(dir)).unwrap();
    }
    std::fs::write(
        store.join("meta.json"),
        format!(r#"{{"session_id":"{id}","title":"Fix login"}}"#),
    )
    .unwrap();
    let nested = [
        store.join("journal/0000000000000001.jsonl"),
        store.join("generations/0000000000000008/checkpoint.json"),
        store.join("generations/0000000000000008/projection-state.json"),
        store.join("chunks/1e8952ce2ea28a31a9dd52cf62f56f15c004ad121e87ae7cb29bf68ab0e19f0e.json"),
    ];
    for file in &nested {
        // projection-state.json carries its own session_id: the store's meta.json still decides.
        std::fs::write(file, br#"{"session_id":"another-chat"}"#).unwrap();
        assert_eq!(
            chat(&folder, file),
            Some((id.to_owned(), "Fix login".to_owned())),
            "{}",
            file.display()
        );
        assert!(sealed(&folder, file), "{}", file.display());
    }
    assert_eq!(chat(&folder, &store.join("meta.json")).unwrap().0, id);
    assert!(sealed(&folder, &store.join("meta.json")));

    // A meta.json that names another chat hands this folder's files to nobody.
    let copied = folder.join("unified/0c0ffee0-0000-0000-0000-000000000000");
    std::fs::create_dir_all(copied.join("chunks")).unwrap();
    std::fs::write(
        copied.join("meta.json"),
        format!(r#"{{"session_id":"{id}"}}"#),
    )
    .unwrap();
    std::fs::write(copied.join("chunks/a.json"), b"[]").unwrap();
    assert_eq!(chat(&folder, &copied.join("chunks/a.json")), None);

    // Vibe 3.0's chat index sits beside the stores: never a chat's file, and not checked.
    let index = folder.join("unified/.session-index.json");
    std::fs::write(&index, br#"{"catalog_version":1,"entries":[]}"#).unwrap();
    assert!(derived_index(&folder, &index));
    assert!(!derived_index(&folder, &store.join("meta.json")));
    assert_eq!(chat(&folder, &index), None);

    // 2.26.1 (0.1.1 users' logs): meta.json beside messages.jsonl, editable.
    let legacy = folder.join("session_20261001_120000_0c0ffee0");
    std::fs::create_dir_all(&legacy).unwrap();
    std::fs::write(
        legacy.join("meta.json"),
        br#"{"session_id":"0c0ffee0-2261","title":"Old chat"}"#,
    )
    .unwrap();
    std::fs::write(legacy.join("messages.jsonl"), b"{}\n").unwrap();
    assert_eq!(
        chat(&folder, &legacy.join("messages.jsonl")),
        Some(("0c0ffee0-2261".to_owned(), "Old chat".to_owned()))
    );
    assert!(!sealed(&folder, &legacy.join("messages.jsonl")));

    // A save_dir that itself lies below a folder named `unified` does not make 2.x files sealed.
    let outer = folder.join("unified-parent/unified/logs");
    std::fs::create_dir_all(outer.join("session_x")).unwrap();
    std::fs::write(outer.join("session_x/messages.jsonl"), b"{}\n").unwrap();
    assert!(!sealed(&outer, &outer.join("session_x/messages.jsonl")));
    // A file that cannot be resolved is treated as sealed (never edited on a guess).
    assert!(sealed(&outer, &outer.join("session_x/missing.jsonl")));
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn title_parts_of_a_value_are_withheld_and_short_overlaps_kept() {
    use crate::secret_scan_cache::withhold_parts;
    let value = "sk-live-4f9Qz7Lm2Xc8Vb1Np6Rt3Yw0";
    assert_eq!(
        withhold_parts("Use sk-live-4f9Qz7Lm2Xc8 now", &[value]),
        "Use [secret withheld] now"
    );
    assert_eq!(withhold_parts("Fix login", &[value]), "Fix login");
    assert_eq!(withhold_parts("live-4f", &[value]), "[secret withheld]");
    assert_eq!(withhold_parts("sk-li", &[value]), "sk-li");
}

#[test]
fn saved_auto_redact_from_an_earlier_version_loads_keeps_scanning_and_is_dropped_on_save() {
    let dir = scratch("older-settings");
    let path = dir.join("settings.json");
    std::fs::write(
        &path,
        br#"{"theme":"dark","secretScan":{"enabled":true,"autoRedact":true}}"#,
    )
    .unwrap();
    assert_eq!(
        crate::app_settings::scan_settings(&path),
        crate::SecretScanSettings { enabled: true }
    );
    crate::write_app_settings(
        &path,
        serde_json::json!({"keepAwake":false})
            .as_object()
            .unwrap()
            .clone(),
    )
    .unwrap();
    let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(
        saved,
        serde_json::json!({"theme":"dark","keepAwake":false,"secretScan":{"enabled":true}})
    );
    std::fs::remove_dir_all(dir).unwrap();
}
