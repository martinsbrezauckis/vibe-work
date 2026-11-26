//! Windows NTFS hard-link and protected-scan regression tests for the shared read policy.
use super::*;

#[test]
fn windows_opened_hard_link_names_keep_the_selected_identity() {
    let (root, policy, paths) = fixture("windows-other-link-path");
    let original = root.join("a.txt");
    let alias = root.join("b.txt");
    let plain = root.join("plain.txt");
    std::fs::write(&original, "ordinary").unwrap();
    std::fs::write(&plain, "plain").unwrap();
    let first = File::open(&original).unwrap();
    std::fs::hard_link(&original, &alias).unwrap();
    for (file, selected) in [(&first, &original), (&first, &alias)] {
        let reported = vw_platform::final_path::final_path(file).unwrap();
        println!(
            "WINSEARCH_CHECK_OPENED selected={} reported={}",
            selected.display(),
            reported.display()
        );
        assert_eq!(policy.check_opened(file, selected, None), Ok(()));
    }
    let second = File::open(&alias).unwrap();
    for selected in [&original, &alias] {
        let reported = vw_platform::final_path::final_path(&second).unwrap();
        println!(
            "WINSEARCH_CHECK_OPENED selected={} reported={}",
            selected.display(),
            reported.display()
        );
        assert_eq!(policy.check_opened(&second, selected, None), Ok(()));
    }
    assert_eq!(
        policy.check_opened(&File::open(&plain).unwrap(), &plain, None),
        Ok(())
    );
    let protected_alias = root.join("protected-alias.txt");
    std::fs::hard_link(&paths[0], &protected_alias).unwrap();
    assert_eq!(
        policy.check_opened(
            &File::open(&protected_alias).unwrap(),
            &protected_alias,
            None
        ),
        Err(PROTECTED.into())
    );
    std::fs::remove_file(&alias).unwrap();
    std::fs::write(&alias, "replacement").unwrap();
    assert!(policy.check_opened(&second, &alias, None).is_err());
    drop(second);
    drop(first);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn windows_protected_scan_uses_metadata_for_an_exclusively_held_file() {
    use std::os::windows::fs::OpenOptionsExt;

    let (root, policy, paths) = fixture("windows-held-protected-entry");
    let held_path = root.join("app-data/cache/held.db");
    std::fs::create_dir_all(held_path.parent().unwrap()).unwrap();
    std::fs::write(&held_path, "held app state").unwrap();
    let held = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(0)
        .open(&held_path)
        .unwrap();
    assert!(
        File::open(&held_path).is_err(),
        "ordinary read unexpectedly bypassed the sharing restriction"
    );
    let metadata_only = vw_platform::final_path::open_metadata(&held_path).unwrap();
    println!(
        "WINSEARCH_HELD protected={} metadata_identity={:?}",
        held_path.display(),
        vw_platform::final_path::identity(&metadata_only).unwrap()
    );
    let safe = root.join("a.txt");
    let alias = root.join("b.txt");
    std::fs::write(&safe, "safe").unwrap();
    std::fs::hard_link(&safe, &alias).unwrap();
    assert_eq!(
        policy.check_opened(&File::open(&alias).unwrap(), &alias, None),
        Ok(())
    );
    drop(metadata_only);
    drop(held);
    let protected_alias = root.join("protected-alias.txt");
    std::fs::hard_link(&paths[0], &protected_alias).unwrap();
    assert_eq!(
        policy.check_opened(
            &File::open(&protected_alias).unwrap(),
            &protected_alias,
            None
        ),
        Err(PROTECTED.into())
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn windows_junction_cannot_turn_a_changed_hard_link_name_into_a_match() {
    let (root, policy, _) = fixture("windows-junction-hard-link");
    let actual = root.join("actual");
    std::fs::create_dir(&actual).unwrap();
    let first = actual.join("a.txt");
    let second = actual.join("b.txt");
    std::fs::write(&first, "safe").unwrap();
    let opened = File::open(&first).unwrap();
    std::fs::hard_link(&first, &second).unwrap();
    let junction = root.join("junction");
    let status = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&junction)
        .arg(&actual)
        .status()
        .unwrap();
    assert!(status.success());
    let selected = junction.join("b.txt");
    assert_ne!(
        vw_platform::plain_path(&selected.canonicalize().unwrap()),
        selected
    );
    assert!(policy.check_opened(&opened, &selected, None).is_err());
    drop(opened);
    std::fs::remove_dir(&junction).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}
