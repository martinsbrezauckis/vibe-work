//! Scanner policy and incremental falsifiers; no runtime Vault or user log files are read.
//! Depends on scheduling/cache helpers and invented inputs; complements dispatcher integration tests.

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
    assert!(cache.changed(&path, &stamp, &fingerprint, false));
    cache.files.insert(
        path.clone(),
        CachedFile {
            stamp: stamp.clone(),
            fingerprint: fingerprint.clone(),
            chat_id: "chat".to_owned(),
            title: "Fix login".to_owned(),
            hits: BTreeMap::<String, SecretLeakName>::new(),
            places: 0,
            retry_auto: false,
        },
    );
    assert!(!cache.changed(&path, &stamp, &fingerprint, false));
    assert!(cache.changed(&path, &Stamp { size: 101, ..stamp }, &fingerprint, false));
    assert!(cache.changed(
        &path,
        &Stamp {
            modified_ns: 43,
            ..stamp
        },
        &fingerprint,
        false
    ));
    assert!(cache.changed(&path, &stamp, "changed-secret-set", false));
    cache.files.get_mut(&path).unwrap().retry_auto = true;
    assert!(cache.changed(&path, &stamp, &fingerprint, true));
    let text = serde_json::to_string(&cache).unwrap();
    assert!(!text.contains("invented-value-a"));
    assert!(!text.contains("invented-value-b"));
}
