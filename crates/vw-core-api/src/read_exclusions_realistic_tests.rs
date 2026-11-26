//! Real-install-sized protected trees exercise metadata scans, agent search and fail-closed budgets.
//! Uses synthetic profiles only; called by the read-exclusion test module on Unix.
use super::*;

fn realistic_tree(root: &Path, count: usize) {
    let cache = root.join("app-data/venv");
    std::fs::create_dir_all(&cache).unwrap();
    for index in 0..count {
        std::fs::write(cache.join(format!("package-{index}.py")), "single link").unwrap();
    }
    let protected = root.join("app-data/state/profile.json");
    std::fs::hard_link(&protected, cache.join("linked-profile.json")).unwrap();
}

#[test]
fn safe_hard_links_in_realistic_protected_tree_are_searchable() {
    let (root, mut policy, _) = fixture("realistic-safe");
    realistic_tree(&root, 50_000);
    // Use the production clock, including on a shared, loaded test host.
    policy.clock = None;
    let project = root.join("project");
    std::fs::create_dir(&project).unwrap();
    let original = project.join("a.txt");
    let alias = project.join("b.txt");
    std::fs::write(&original, "HARDLINKS3_REALISTIC_MARKER").unwrap();
    std::fs::hard_link(&original, &alias).unwrap();
    let started = Instant::now();
    let decision = policy.check_opened(&File::open(&alias).unwrap(), &alias, None);
    println!(
        "HARDLINKS3 50000-entry policy scan: {:?}; result={decision:?}",
        started.elapsed()
    );
    let search_started = Instant::now();
    let deadline = search_started + Duration::from_secs(4);
    let search = crate::search_agent::run(
        &[crate::search_files::Folder {
            path: project,
            session_id: None,
        }],
        &serde_json::from_value(serde_json::json!({"query":"HARDLINKS3_REALISTIC_MARKER"}))
            .unwrap(),
        &|path| !policy.excludes(path),
        &|file, named| policy.check_opened(file, named, Some(deadline)).is_ok(),
        Duration::from_secs(4),
    )
    .unwrap();
    println!(
        "HARDLINKS3 agent search ({:?}): {search}",
        search_started.elapsed()
    );
    std::fs::remove_dir_all(root).unwrap();
    assert_eq!(decision, Ok(()));
    assert!(
        search.contains("a.txt:1: HARDLINKS3_REALISTIC_MARKER"),
        "{search}"
    );
    assert!(
        search.contains("b.txt:1: HARDLINKS3_REALISTIC_MARKER"),
        "{search}"
    );
}

#[test]
fn protected_hard_links_in_realistic_tree_are_refused() {
    let (root, policy, paths) = fixture("realistic-protected");
    realistic_tree(&root, 15_000);
    let alias = root.join("project-profile.txt");
    std::fs::hard_link(&paths[0], &alias).unwrap();
    let decision = policy.check_opened(&File::open(&alias).unwrap(), &alias, None);
    std::fs::remove_dir_all(root).unwrap();
    assert_eq!(decision, Err(PROTECTED.into()));
}

#[test]
fn an_injected_clock_exhausts_the_scan_budget() {
    let (root, mut policy, _) = fixture("realistic-clock");
    let ordinary = root.join("a.txt");
    let alias = root.join("b.txt");
    std::fs::write(&ordinary, "safe").unwrap();
    std::fs::hard_link(&ordinary, &alias).unwrap();
    let checks = std::sync::atomic::AtomicUsize::new(0);
    let now = Instant::now();
    policy.clock = Some(std::sync::Arc::new(move || {
        if checks.fetch_add(1, std::sync::atomic::Ordering::SeqCst) < 5 {
            now
        } else {
            now + IDENTITY_BUDGET
        }
    }));
    let decision = policy.check_opened(&File::open(&alias).unwrap(), &alias, None);
    std::fs::remove_dir_all(root).unwrap();
    assert_eq!(decision, Err(PROTECTED.into()));
}

#[test]
fn unreadable_protected_contents_still_have_usable_link_metadata() {
    use std::os::unix::fs::PermissionsExt;

    let (root, policy, paths) = fixture("realistic-unreadable");
    let original = root.join("a.txt");
    let alias = root.join("b.txt");
    std::fs::write(&original, "safe").unwrap();
    std::fs::hard_link(&original, &alias).unwrap();
    std::fs::hard_link(&paths[0], root.join("selected/second-name.json")).unwrap();
    std::fs::set_permissions(&paths[0], std::fs::Permissions::from_mode(0o000)).unwrap();
    assert!(File::open(&paths[0]).is_err());
    let safe = policy.check_opened(&File::open(&alias).unwrap(), &alias, None);
    let protected_alias = root.join("protected.txt");
    std::fs::hard_link(&paths[0], &protected_alias).unwrap();
    // Its bytes remain unreadable, so use the safe candidate's identity against the protected walk;
    // after restoring read permission the protected alias must still be refused.
    std::fs::set_permissions(&paths[0], std::fs::Permissions::from_mode(0o600)).unwrap();
    let protected = policy.check_opened(
        &File::open(&protected_alias).unwrap(),
        &protected_alias,
        None,
    );
    std::fs::remove_dir_all(root).unwrap();
    assert_eq!(safe, Ok(()));
    assert_eq!(protected, Err(PROTECTED.into()));
}
