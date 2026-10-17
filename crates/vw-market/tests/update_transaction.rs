//! Plugin replacement tests over real folders and receipts, called by cargo test.
//! Depends on `Store` and `std::fs`; a failed final rename must preserve the usable install.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use vw_market::{MarketError, RECEIPT, Receipt, Store, sha256_hex};

struct Home(PathBuf);

impl Home {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "market-update-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn stage(store: &Store, version: &str) -> (PathBuf, Receipt) {
    let path = store.begin_staging("notes-pack").unwrap();
    let manifest = serde_json::json!({"name": "notes-pack", "version": version}).to_string();
    let skill = format!("---\nname: notes-guide\ndescription: Notes\n---\nVersion {version}\n");
    let files = BTreeMap::from([
        ("plugin.json".to_owned(), sha256_hex(manifest.as_bytes())),
        (
            "skills/notes-guide/SKILL.md".to_owned(),
            sha256_hex(skill.as_bytes()),
        ),
    ]);
    std::fs::create_dir_all(path.join("skills/notes-guide")).unwrap();
    std::fs::write(path.join("plugin.json"), manifest).unwrap();
    std::fs::write(path.join("skills/notes-guide/SKILL.md"), skill).unwrap();
    let receipt = Receipt {
        entry_id: "notes-pack".to_owned(),
        version: version.to_owned(),
        catalog_version: "1".to_owned(),
        installed_at_ms: 0,
        files,
    };
    std::fs::write(path.join(RECEIPT), serde_json::to_vec(&receipt).unwrap()).unwrap();
    (path, receipt)
}

fn assert_usable(store: &Store, receipt: &Receipt) {
    assert_eq!(store.receipt("notes-pack").unwrap(), Some(receipt.clone()));
    assert!(
        !store.changed(receipt),
        "installed files must match the old receipt"
    );
    let path = store.plugin_dir("notes-pack").unwrap();
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path.join("plugin.json")).unwrap()).unwrap();
    assert_eq!(manifest["version"], receipt.version);
    assert!(
        std::fs::read_to_string(path.join("skills/notes-guide/SKILL.md"))
            .unwrap()
            .contains(&format!("Version {}", receipt.version))
    );
}

#[test]
fn a_failed_final_rename_keeps_the_old_plugin_usable() {
    let home = Home::new();
    let store = Store::new(&home.0);
    let (old, receipt) = stage(&store, "1");
    store.commit("notes-pack", &old).unwrap();
    let (new, _) = stage(&store, "2");
    // Remove the source of the final rename after building it, injecting NotFound without a test hook.
    std::fs::rename(&new, home.0.join("unavailable-staging")).unwrap();
    let error = store.commit("notes-pack", &new).unwrap_err();
    assert!(
        matches!(
            error,
            MarketError::Io { ref step, ref source }
                if step == "move into plugins/" && source.kind() == std::io::ErrorKind::NotFound
        ),
        "{error}"
    );
    assert_usable(&store, &receipt);
}

#[test]
fn a_normal_update_replaces_the_plugin() {
    let home = Home::new();
    let store = Store::new(&home.0);
    let (old, _) = stage(&store, "1");
    store.commit("notes-pack", &old).unwrap();
    let (new, receipt) = stage(&store, "2");
    store.commit("notes-pack", &new).unwrap();
    assert_usable(&store, &receipt);
    assert!(!new.exists());
    assert!(!home.0.join("vw-market/.aside/notes-pack").exists());
}
