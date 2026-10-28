//! Verifies preview retry deletes only its profile, beside an immutable Vibe store.
//! Depends on the real retry path and std filesystem fixtures; no window.

use crate::readonly_fixture as fixture;

#[test]
fn readonly_vibe_packages_survive_preview_retry() {
    let root = std::env::temp_dir().join(format!("vw-ro1-preview-{}", std::process::id()));
    let checkout = root.join("vibe-home/logs/session/plugins/packages/ab/hash");
    fixture::readonly_package(&checkout);
    let profile = root.join("preview/profile");
    std::fs::create_dir_all(&profile).unwrap();
    let list = root.join("preview/wipe-pending.json");
    super::record_wipes(&list, std::slice::from_ref(&profile));
    super::retry_wipes(&list);
    assert!(!profile.exists());
    assert!(!list.exists());
    assert!(checkout.join("skills/example/SKILL.md").exists());
    vw_platform::remove_owned_tree(root.parent().unwrap(), &root).unwrap();
}
