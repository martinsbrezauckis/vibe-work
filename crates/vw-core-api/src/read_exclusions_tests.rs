//! Synthetic vault locations for the shared read policy and both byte loaders. No real profile is inspected.
use super::*;

#[cfg(unix)]
#[path = "read_exclusions_realistic_tests.rs"]
mod realistic;

#[cfg(target_os = "macos")]
#[test]
fn another_hard_link_path_is_allowed_only_for_the_opened_identity() {
    let (root, policy, paths) = fixture("other-link-path");
    let original = root.join("ordinary.txt");
    let alias = root.join("alias.txt");
    std::fs::write(&original, "ordinary").unwrap();
    let opened = File::open(&original).unwrap();
    std::fs::hard_link(&original, &alias).unwrap();
    assert_ne!(vw_platform::final_path::final_path(&opened).unwrap(), alias);
    assert_eq!(policy.check_opened(&opened, &alias, None), Ok(()));
    let mut bytes = String::new();
    std::io::Read::read_to_string(&mut &opened, &mut bytes).unwrap();
    assert_eq!(bytes, "ordinary");

    let replacement = root.join("replacement.txt");
    std::fs::write(&replacement, "replacement").unwrap();
    assert!(policy.check_opened(&opened, &replacement, None).is_err());
    assert!(policy.check_opened(&opened, Path::new(""), None).is_err());
    assert!(
        policy
            .check_opened(&opened, &root.join("missing"), None)
            .is_err()
    );
    assert_eq!(
        policy.check_opened(&File::open(&paths[0]).unwrap(), &alias, None),
        Err(PROTECTED.into())
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(windows)]
#[path = "read_exclusions_windows_tests.rs"]
mod windows;

#[cfg(target_os = "macos")]
#[test]
fn an_expired_budget_refuses_another_hard_link_path() {
    let (root, policy, _) = fixture("other-link-expired");
    let original = root.join("ordinary.txt");
    let alias = root.join("alias.txt");
    std::fs::write(&original, "ordinary").unwrap();
    let opened = File::open(&original).unwrap();
    std::fs::hard_link(&original, &alias).unwrap();
    assert_ne!(vw_platform::final_path::final_path(&opened).unwrap(), alias);
    assert_eq!(
        policy.check_opened(&opened, &alias, Some(policy.now())),
        Err(PROTECTED.into())
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn an_expired_identity_deadline_refuses_hard_links_but_allows_single_links() {
    let (root, policy, _) = fixture("deadline");
    let ordinary = root.join("ordinary.txt");
    std::fs::write(&ordinary, "ordinary").unwrap();
    let deadline = Some(policy.now());
    policy
        .check_opened(&File::open(&ordinary).unwrap(), &ordinary, deadline)
        .unwrap();
    let alias = root.join("alias.txt");
    std::fs::hard_link(&ordinary, &alias).unwrap();
    let refusal = policy.check_opened(&File::open(&alias).unwrap(), &alias, deadline);
    assert_eq!(
        refusal,
        Err(PROTECTED.into()),
        "an expired budget cleared a multiply linked identity"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_large_protected_tree_refuses_uncleared_hard_links_but_allows_single_links() {
    let (root, mut policy, _) = fixture("bounded");
    large_protected_tree(&root, &mut policy);
    let ordinary = root.join("ordinary.txt");
    std::fs::write(&ordinary, "ordinary").unwrap();
    policy
        .check_opened(&File::open(&ordinary).unwrap(), &ordinary, None)
        .unwrap();
    let alias = root.join("alias.txt");
    std::fs::hard_link(&ordinary, &alias).unwrap();
    assert_eq!(
        policy.check_opened(&File::open(&alias).unwrap(), &alias, None),
        Err(PROTECTED.into()),
        "unbounded walk cleared a hard link beyond the entry cap"
    );
    std::fs::remove_dir_all(root).unwrap();
}

pub(crate) fn large_protected_tree(root: &Path, policy: &mut ReadExclusions) {
    policy.entry_limit = Some(4096);
    let cache = root.join("app-data/cache");
    std::fs::create_dir_all(&cache).unwrap();
    // Empty directories count as work too, even though none supplies a file identity.
    for index in 0..4096 {
        std::fs::create_dir(cache.join(index.to_string())).unwrap();
    }
}

#[cfg(any(unix, windows))]
#[test]
fn protected_symlinks_do_not_own_outside_hard_links_but_direct_files_do() {
    let (root, mut policy, _) = fixture("symlinks");
    let work = count_scan_work(&mut policy);
    let outside = root.join("outside");
    std::fs::create_dir(&outside).unwrap();
    let original = outside.join("ordinary.txt");
    let alias = root.join("alias.txt");
    std::fs::write(&original, "ordinary").unwrap();
    std::fs::hard_link(&original, &alias).unwrap();
    // A nontrivial protected tree keeps the alias decision independent of scheduler speed.
    for index in 0..128 {
        std::fs::create_dir_all(root.join("app-data/cache").join(index.to_string())).unwrap();
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&original, root.join("app-data/linked-file")).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("app-data/linked-directory")).unwrap();
    }
    #[cfg(windows)]
    {
        let test = "protected_symlinks_do_not_own_outside_hard_links_but_direct_files_do";
        if !test_link_created(
            std::os::windows::fs::symlink_file(&original, root.join("app-data/linked-file")),
            test,
        ) || !test_link_created(
            std::os::windows::fs::symlink_dir(&outside, root.join("app-data/linked-directory")),
            test,
        ) {
            std::fs::remove_dir_all(root).unwrap();
            return;
        }
    }
    policy
        .check_opened(&File::open(&alias).unwrap(), &alias, None)
        .expect("a protected symlink made an outside file protected");
    std::fs::hard_link(&original, root.join("app-data/direct.txt")).unwrap();
    assert_eq!(
        policy.check_opened(&File::open(&alias).unwrap(), &alias, None),
        Err(PROTECTED.into())
    );
    let steps = work.load(std::sync::atomic::Ordering::SeqCst);
    println!(
        "protected symlink/direct-file decisions: {steps} scan budget checks for 128 cache entries"
    );
    assert!(
        steps <= 8 * (128 + 16),
        "the bounded tree was repeatedly scanned: {steps}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn hard_links_to_every_protected_location_are_refused_but_ordinary_links_are_allowed() {
    let (root, policy, paths) = fixture("hard-links");
    for (i, path) in paths.iter().enumerate() {
        let alias = root.join(format!("notes-{i}.json"));
        if !test_link_created(
            std::fs::hard_link(path, &alias),
            "hard_links_to_every_protected_location_are_refused_but_ordinary_links_are_allowed",
        ) {
            std::fs::remove_dir_all(root).unwrap();
            return;
        }
        let error = policy
            .check_opened(&File::open(&alias).unwrap(), &alias, None)
            .expect_err("hard-linked protected file was allowed");
        assert!(
            error.contains("vault or app state") && error.contains("Choose a file"),
            "{error}"
        );
    }
    let original = root.join("ordinary.json");
    let alias = root.join("copy.json");
    std::fs::write(&original, "ordinary").unwrap();
    policy
        .check_opened(&File::open(&original).unwrap(), &original, None)
        .unwrap();
    if !test_link_created(
        std::fs::hard_link(&original, &alias),
        "hard_links_to_every_protected_location_are_refused_but_ordinary_links_are_allowed",
    ) {
        std::fs::remove_dir_all(root).unwrap();
        return;
    }
    policy
        .check_opened(&File::open(&alias).unwrap(), &alias, None)
        .unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(any(unix, windows))]
#[test]
fn native_alias_uses_the_same_roots_and_allows_safe_and_single_links() {
    let (root, policy, paths) = fixture("native-alias-roots");
    for (index, protected) in paths.iter().enumerate() {
        let alias = root.join(format!("native-alias-{index}.txt"));
        std::fs::hard_link(protected, &alias).unwrap();
        assert_eq!(policy.check_native_alias(&alias), Err(PROTECTED.into()));
    }
    let safe = root.join("safe.txt");
    let safe_alias = root.join("safe-alias.txt");
    std::fs::write(&safe, "ordinary").unwrap();
    assert_eq!(policy.check_native_alias(&safe), Ok(()));
    std::fs::hard_link(&safe, &safe_alias).unwrap();
    assert_eq!(policy.check_native_alias(&safe_alias), Ok(()));
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(any(unix, windows))]
#[test]
fn native_alias_budget_exhaustion_refuses_but_single_link_skips_the_walk() {
    let (root, mut policy, _) = fixture("native-alias-budget");
    policy.entry_limit = Some(3);
    let safe = root.join("safe.txt");
    let alias = root.join("alias.txt");
    std::fs::write(&safe, "ordinary").unwrap();
    assert_eq!(
        policy.check_native_alias(&root.join("removed-after-hook.txt")),
        Err(PROTECTED.into())
    );
    assert_eq!(policy.check_native_alias(&safe), Ok(()));
    std::fs::hard_link(&safe, &alias).unwrap();
    assert_eq!(policy.check_native_alias(&alias), Err(PROTECTED.into()));
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn native_alias_walk_is_bounded_on_a_large_protected_tree() {
    let (root, mut policy, _) = fixture("native-alias-performance");
    policy.clock = None;
    let cache = root.join("app-data/cache");
    std::fs::create_dir_all(&cache).unwrap();
    for index in 0..10_000 {
        std::fs::write(cache.join(index.to_string()), b"x").unwrap();
    }
    let safe = root.join("safe.txt");
    let alias = root.join("alias.txt");
    std::fs::write(&safe, b"ordinary").unwrap();
    std::fs::hard_link(&safe, &alias).unwrap();
    let started = Instant::now();
    let answer = policy.check_native_alias(&alias);
    let elapsed = started.elapsed();
    println!("native alias scan: 10000 single-link entries in {elapsed:?}, decision={answer:?}");
    assert!(
        elapsed < Duration::from_secs(5),
        "walk exceeded the 3 s budget plus scheduler margin"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(any(unix, windows))]
#[test]
fn native_grep_directory_refuses_protected_alias_but_allows_safe_alias() {
    let (root, policy, paths) = fixture("native-grep-directory");
    let project = root.join("project");
    std::fs::create_dir_all(&project).unwrap();
    let safe = root.join("safe.txt");
    std::fs::write(&safe, "ordinary").unwrap();
    std::fs::hard_link(&safe, project.join("safe-alias.txt")).unwrap();
    assert_eq!(policy.check_native_search(&project), Ok(()));
    std::fs::hard_link(&paths[0], project.join("protected-alias.txt")).unwrap();
    assert_eq!(policy.check_native_search(&project), Err(PROTECTED.into()));
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(windows)]
#[test]
fn native_grep_directory_budget_exhaustion_refuses() {
    let (root, mut policy, _) = fixture("native-grep-budget");
    let project = root.join("project");
    std::fs::create_dir_all(&project).unwrap();
    let safe = root.join("safe.txt");
    std::fs::write(&safe, "ordinary").unwrap();
    std::fs::hard_link(&safe, project.join("safe-alias.txt")).unwrap();
    policy.entry_limit = Some(1);
    assert_eq!(policy.check_native_search(&project), Err(PROTECTED.into()));
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(windows)]
#[test]
fn native_identity_checks_exclusively_held_files_without_a_false_refusal() {
    use std::os::windows::fs::OpenOptionsExt;

    let (root, policy, paths) = fixture("native-exclusive-handles");
    let project = root.join("project");
    std::fs::create_dir_all(&project).unwrap();
    let safe = project.join("safe.txt");
    let safe_alias = project.join("safe-alias.txt");
    std::fs::write(&safe, "ordinary").unwrap();
    std::fs::hard_link(&safe, &safe_alias).unwrap();
    let held_safe = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&safe)
        .unwrap();
    assert_eq!(policy.check_native_alias(&safe), Ok(()));
    assert_eq!(policy.check_native_alias(&safe_alias), Ok(()));
    assert_eq!(policy.check_native_search(&project), Ok(()));
    drop(held_safe);

    let protected_alias = project.join("protected-alias.txt");
    std::fs::hard_link(&paths[0], &protected_alias).unwrap();
    let held_protected = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&paths[0])
        .unwrap();
    assert_eq!(policy.check_native_alias(&safe_alias), Ok(()));
    assert_eq!(
        policy.check_native_alias(&protected_alias),
        Err(PROTECTED.into())
    );
    assert_eq!(policy.check_native_search(&project), Err(PROTECTED.into()));
    drop(held_protected);
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(windows)]
#[test]
#[ignore = "measured native NTFS directory grep benchmark"]
fn native_grep_twenty_thousand_hard_links_is_bounded() {
    let (root, mut policy, _) = fixture("native-grep-realistic");
    policy.clock = None;
    let project = root.join("project");
    std::fs::create_dir_all(&project).unwrap();
    for group in 0..20 {
        let safe = project.join(format!("safe-{group}.txt"));
        std::fs::write(&safe, "ordinary").unwrap();
        for index in 0..999 {
            std::fs::hard_link(&safe, project.join(format!("alias-{group}-{index}.txt"))).unwrap();
        }
    }
    let start = Instant::now();
    let result = policy.check_native_search(&project);
    println!(
        "native grep: 20000 NTFS files with hard links in {:?}, decision={result:?}",
        start.elapsed()
    );
    assert_eq!(result, Ok(()));
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(windows)]
#[test]
#[ignore = "measured native NTFS plain directory grep benchmark"]
fn native_grep_two_thousand_plain_files_timing() {
    let (root, mut policy, _) = fixture("native-grep-plain-2000");
    policy.clock = None;
    let project = root.join("project");
    std::fs::create_dir_all(&project).unwrap();
    for index in 0..2_000 {
        std::fs::write(project.join(format!("file-{index}.txt")), b"ordinary").unwrap();
    }
    let start = Instant::now();
    let result = policy.check_native_search(&project);
    println!(
        "native grep: 2000 plain NTFS files in {:?}, decision={result:?}",
        start.elapsed()
    );
    assert_eq!(result, Ok(()));
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(windows)]
#[test]
#[ignore = "measured native NTFS node_modules grep benchmark"]
fn native_grep_real_node_modules_timing() {
    let tree = PathBuf::from(
        std::env::var_os("VW_NATIVE_GREP_TREE").expect("set a real Windows node_modules tree"),
    );
    let count = walkdir::WalkDir::new(&tree)
        .follow_links(false)
        .follow_root_links(false)
        .into_iter()
        .count();
    let (root, mut policy, _) = fixture("native-grep-node-modules");
    policy.clock = None;
    let start = Instant::now();
    let result = policy.check_native_search(&tree);
    println!(
        "native grep: real node_modules entries={count} in {:?}, decision={result:?}",
        start.elapsed()
    );
    std::fs::remove_dir_all(root).unwrap();
    assert_eq!(result, Ok(()));
}

// Windows can still identify an exclusively held file through open_metadata; its counterpart is
// windows::windows_protected_scan_uses_metadata_for_an_exclusively_held_file.
#[cfg(unix)]
#[test]
fn an_unavailable_protected_identity_refuses_hard_links_but_does_not_scan_single_links() {
    use std::os::unix::fs::PermissionsExt;

    let (root, policy, paths) = fixture("unavailable");
    let ordinary = root.join("ordinary.txt");
    std::fs::write(&ordinary, "ordinary").unwrap();
    std::fs::set_permissions(
        paths[0].parent().unwrap(),
        std::fs::Permissions::from_mode(0o000),
    )
    .unwrap();
    policy
        .check_opened(&File::open(&ordinary).unwrap(), &ordinary, None)
        .unwrap();
    let alias = root.join("alias.txt");
    std::fs::hard_link(&ordinary, &alias).unwrap();
    let error = policy
        .check_opened(&File::open(&alias).unwrap(), &alias, None)
        .unwrap_err();
    assert_eq!(error, PROTECTED);
    std::fs::set_permissions(
        paths[0].parent().unwrap(),
        std::fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

pub(crate) fn fixture(name: &str) -> (PathBuf, ReadExclusions, Vec<PathBuf>) {
    let root = std::env::temp_dir().join(format!("read-policy-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let root = vw_platform::canonical_plain(&root).unwrap();
    let data = root.join("app-data");
    let home = data.join("vibe");
    let selected = root.join("selected");
    let input = vw_vault::location::ProfileInput {
        platform: vw_vault::location::Platform::Linux,
        home: Some(root.clone()),
        xdg_config_home: Some(root.join("config")),
        appdata: None,
        override_dir: None,
    };
    let shared = vw_vault::location::resolve(&input).unwrap();
    let paths: Vec<_> = [
        vw_vault::private_location::private_vault_dir(&data).unwrap(),
        selected.clone(),
        shared.canonical,
        shared.legacy,
        data.join("vault"),
        data.join("state"),
    ]
    .into_iter()
    .map(|dir| {
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("profile.json");
        std::fs::write(&path, br#"{"token":"synthetic-profile"}"#).unwrap();
        path
    })
    .collect();
    let mut policy = ReadExclusions::new(&home, &root, Some(selected), &input).unwrap();
    count_scan_work(&mut policy);
    (root, policy, paths)
}

/// Count the scan's per-root/entry/identity budget checks with a fixed monotonic clock. Production keeps its
/// real platform deadline; these policy tests measure traversal work even when the scheduler suspends the process.
fn count_scan_work(policy: &mut ReadExclusions) -> std::sync::Arc<std::sync::atomic::AtomicUsize> {
    let work = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed = work.clone();
    let now = Instant::now();
    policy.clock = Some(std::sync::Arc::new(move || {
        observed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        now
    }));
    work
}

#[test]
fn every_vault_location_and_app_state_are_excluded() {
    let (root, policy, paths) = fixture("paths");
    for path in paths {
        assert!(policy.excludes(&path), "{}", path.display());
        let error = policy
            .check_opened(&File::open(&path).unwrap(), &path, None)
            .unwrap_err();
        assert!(error.contains("vault or app state") && error.contains("Choose a file"));
    }
    assert!(!policy.excludes(&root.join("ordinary.json")));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn the_opened_identity_is_checked_even_when_the_original_name_was_allowed() {
    let (root, policy, paths) = fixture("opened");
    let named = root.join("ordinary.json");
    std::fs::write(&named, "{}").unwrap();
    let opened = File::open(&paths[0]).unwrap();
    let error = policy.check_opened(&opened, &named, None).unwrap_err();
    assert!(error.contains("vault or app state"), "{error}");
    let original = File::open(&named).unwrap();
    std::fs::rename(&named, root.join("old.json")).unwrap();
    std::fs::write(&named, "replacement").unwrap();
    assert!(policy.check_opened(&original, &named, None).is_err());
    drop(original);
    drop(opened);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn only_the_own_copy_is_exempt_and_a_vault_inside_it_is_still_excluded() {
    let (root, _, _) = fixture("copy");
    let home = root.join("app-data/vibe");
    let own = home.join("worktrees/repo/own");
    let vault = own.join("vault");
    let policy = ReadExclusions::new(
        &home,
        &own,
        Some(vault.clone()),
        &vw_vault::location::ProfileInput::from_env(),
    )
    .unwrap();
    assert!(!policy.excludes(&own.join("src/a")));
    assert!(policy.excludes(&vault.join("profile.json")));
    assert!(policy.excludes(&own.with_file_name("other").join("src/a")));
    std::fs::remove_dir_all(root).unwrap();
}

pub(crate) fn test_link_created(result: std::io::Result<()>, test: &str) -> bool {
    link_created_for_gate(
        result,
        test,
        std::env::var("VW_REQUIRE_ALL").is_ok_and(|value| value == "1"),
    )
}

fn link_created_for_gate(result: std::io::Result<()>, test: &str, required: bool) -> bool {
    match result {
        Ok(()) => true,
        Err(error) if error.raw_os_error() == Some(1314) => {
            use std::io::Write;
            // Write directly so libtest's normal output capture cannot hide the skip.
            writeln!(std::io::stderr(), "NOT RUN {test}: Windows link privilege is missing (ERROR_PRIVILEGE_NOT_HELD, 1314).").unwrap();
            assert!(
                !required,
                "{test}: Windows link privilege is required; enable Developer Mode or run the native gate with link privilege"
            );
            false
        }
        Err(error) => panic!("{test}: link fixture failed: {error}"),
    }
}

#[test]
fn missing_link_privilege_is_reported_as_a_named_skip() {
    assert!(!link_created_for_gate(
        Err(std::io::Error::from_raw_os_error(1314)),
        "synthetic missing-privilege case",
        false
    ));
    assert!(test_link_created(Ok(()), "synthetic permitted case"));
}

#[test]
#[should_panic(expected = "link fixture failed")]
fn other_link_errors_are_failures() {
    test_link_created(
        Err(std::io::Error::from_raw_os_error(5)),
        "synthetic other-error case",
    );
}

#[cfg(windows)]
#[test]
fn protected_junctions_do_not_own_outside_hard_links_but_direct_files_do() {
    let (root, policy, paths) = fixture("junctions");
    let outside = root.join("outside");
    std::fs::create_dir(&outside).unwrap();
    let original = outside.join("ordinary.txt");
    let alias = root.join("alias.txt");
    std::fs::write(&original, "ordinary").unwrap();
    std::fs::hard_link(&original, &alias).unwrap();
    // Built from components: cmd.exe reads a forward slash in an unquoted argument as the start of a switch.
    let linked_outside = root.join("app-data").join("linked-directory");
    let linked_protected = root.join("protected-alias");
    for (link, target) in [
        (linked_outside.as_path(), outside.as_path()),
        (linked_protected.as_path(), paths[0].parent().unwrap()),
    ] {
        let output = std::process::Command::new("cmd.exe")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "junction fixture failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    policy
        .check_opened(&File::open(&alias).unwrap(), &alias, None)
        .unwrap();
    let protected = linked_protected.join("profile.json");
    assert_eq!(
        policy.check_opened(&File::open(&protected).unwrap(), &protected, None),
        Err(PROTECTED.into())
    );
    std::fs::hard_link(&original, root.join("app-data/direct.txt")).unwrap();
    assert_eq!(
        policy.check_opened(&File::open(&alias).unwrap(), &alias, None),
        Err(PROTECTED.into())
    );
    std::fs::remove_dir(linked_outside).unwrap();
    std::fs::remove_dir(linked_protected).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn missing_link_privilege_fails_a_required_gate() {
    let refused = std::panic::catch_unwind(|| {
        link_created_for_gate(
            Err(std::io::Error::from_raw_os_error(1314)),
            "synthetic required case",
            true,
        )
    });
    assert!(
        refused.is_err(),
        "required link coverage must never become a passing skip"
    );
    assert!(link_created_for_gate(
        Ok(()),
        "synthetic permitted gate",
        true
    ));
}

// PREDICT (red before `overlaps`): only `excludes` existed, which answers for a path INSIDE a protected root, so the
// vault's container (`Vibe Work Vault`, beside the app data on every OS) and every folder above it passed a whole-folder
// check. Runs on each OS with that OS's own path shape (drive letters and letter case on Windows).
#[test]
fn a_folder_holding_the_vault_or_app_data_overlaps_but_a_sibling_does_not() {
    let (root, policy, _) = fixture("overlaps");
    let vault = vw_vault::private_location::private_vault_dir(&root.join("app-data")).unwrap();
    let container = vault.parent().unwrap();
    for folder in [
        container.to_path_buf(),
        root.clone(),
        vault.join("local-vault-server"),
        root.join("app-data"),
        root.join("selected"),
    ] {
        assert!(
            policy.overlaps(&folder),
            "{} was not refused",
            folder.display()
        );
    }
    #[cfg(windows)]
    {
        let shouted = PathBuf::from(container.to_string_lossy().to_uppercase());
        assert!(policy.overlaps(&shouted), "{}", shouted.display());
    }
    let library = root.join("library");
    std::fs::create_dir_all(&library).unwrap();
    assert!(!policy.overlaps(&library), "a sibling folder was refused");
    assert!(
        !policy.excludes(container),
        "a file read beside the vault is not refused by this rule"
    );
    std::fs::remove_dir_all(root).unwrap();
}

// The agent's folder guard names the vault's folder itself (its binary does not link the vault): it must be the
// same folder the vault is created in, for each OS's app data spelling.
#[test]
fn the_folder_guard_reserves_the_same_vault_folder_the_vault_uses() {
    let (root, _, _) = fixture("guard-vault");
    for data in [
        root.join("AppData/Roaming/app.vibework.desktop"),
        root.join("Library/Application Support/app.vibework.desktop"),
        root.join(".local/share/app.vibework.desktop"),
    ] {
        let vault = vw_vault::private_location::private_vault_dir(&data).unwrap();
        assert_eq!(
            vw_agent_tools::folders::private_vault_container(&data).as_deref(),
            vault.parent()
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}
