//! Verifies the real profile delete cannot reach the sibling immutable Vibe store.
//! Depends on storage's retry function and a filesystem fixture; no webview or window.

#[path = "../../../tests/public/fixtures/readonly_package.rs"]
mod fixture;

#[tokio::test]
async fn readonly_vibe_packages_survive_storage_delete() {
    let root = std::env::temp_dir().join(format!("vw-ro1-storage-{}", std::process::id()));
    let checkout = root.join("vibe-home/logs/session/plugins/packages/ab/hash");
    fixture::readonly_package(&checkout);
    let profile = root.join("preview/profile");
    std::fs::create_dir_all(&profile).unwrap();
    super::remove_with_retries(&profile, 1).await;
    assert!(!profile.exists());
    assert!(checkout.join("skills/example/SKILL.md").exists());
    vw_platform::remove_owned_tree(root.parent().unwrap(), &root).unwrap();
}
