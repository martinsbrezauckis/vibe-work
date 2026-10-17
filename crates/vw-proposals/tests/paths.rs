//! Path-boundary fixtures using the public API and isolated tempdirs.
//! Falsifiers cover traversal, absolute paths and symlinks at capture and approval.

#![allow(clippy::unwrap_used)]

use std::fs;
use std::path::{Path, PathBuf};

use tempfile::TempDir;
#[cfg(unix)]
use vw_proposals::Status;
use vw_proposals::{Change, Decision, Error, Proposal, Request};

fn request(changes: Vec<Change>) -> Request {
    Request {
        session_id: "paths".into(),
        reason: "Update profiles".into(),
        changes,
    }
}

fn add(path: impl Into<PathBuf>) -> Change {
    Change::Add {
        path: path.into(),
        content: b"proposed".to_vec(),
    }
}

#[test]
fn traversal_absolute_and_empty_paths_are_refused() {
    let root = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    for path in [
        PathBuf::from("../escape"),
        "profiles/../../escape".into(),
        outside.path().join("absolute"),
        "".into(),
        ".".into(),
    ] {
        assert!(matches!(
            Proposal::new(root.path(), request(vec![add(path)])),
            Err(Error::InvalidPath { .. })
        ));
    }
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
}

#[test]
fn dot_components_normalize_but_duplicate_targets_are_refused() {
    let root = TempDir::new().unwrap();
    let proposal = Proposal::new(root.path(), request(vec![add("./new")])).unwrap();
    assert_eq!(proposal.review()[0].path, Path::new("new"));
    let duplicate = Proposal::new(root.path(), request(vec![add("new"), add("./new")]));
    assert!(matches!(duplicate, Err(Error::DuplicatePath { .. })));
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn directories_are_not_files_and_missing_parents_are_not_created() {
    let root = TempDir::new().unwrap();
    fs::create_dir(root.path().join("directory")).unwrap();
    for change in [
        Change::Edit {
            path: "directory".into(),
            content: b"overwrite".to_vec(),
        },
        Change::Remove {
            path: "directory".into(),
        },
    ] {
        assert!(matches!(
            Proposal::new(root.path(), request(vec![change])),
            Err(Error::WrongType { .. })
        ));
    }
    assert!(matches!(
        Proposal::new(root.path(), request(vec![add("missing/new")])),
        Err(Error::Io { .. })
    ));
    assert!(!root.path().join("missing").exists());
    assert!(root.path().join("directory").is_dir());
}

#[test]
fn directory_replacing_a_base_is_stale_before_any_write() {
    let root = TempDir::new().unwrap();
    fs::write(root.path().join("base"), b"original").unwrap();
    let proposal = Proposal::new(
        root.path(),
        request(vec![
            add("new"),
            Change::Remove {
                path: "base".into(),
            },
        ]),
    )
    .unwrap();
    fs::remove_file(root.path().join("base")).unwrap();
    fs::create_dir(root.path().join("base")).unwrap();
    let error = proposal.apply(Decision::Approve).unwrap_err();
    assert!(matches!(error.error, Error::Stale { .. }));
    assert!(!root.path().join("new").exists());
    assert!(root.path().join("base").is_dir());
}

#[cfg(unix)]
mod unix {
    use std::os::unix::ffi::OsStringExt;
    use std::os::unix::fs::{PermissionsExt, symlink};

    use super::*;

    #[test]
    fn symlink_targets_and_parents_are_refused_even_when_they_stay_inside() {
        let root = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        fs::create_dir(root.path().join("real")).unwrap();
        fs::write(root.path().join("real/file"), b"original").unwrap();
        symlink(root.path().join("real/file"), root.path().join("file-link")).unwrap();
        symlink(root.path().join("real"), root.path().join("inside-link")).unwrap();
        symlink(outside.path(), root.path().join("outside-link")).unwrap();
        symlink(root.path().join("absent"), root.path().join("dangling")).unwrap();
        for path in ["file-link", "dangling"] {
            for change in [
                add(path),
                Change::Edit {
                    path: path.into(),
                    content: b"proposed".to_vec(),
                },
                Change::Remove { path: path.into() },
            ] {
                assert!(matches!(
                    Proposal::new(root.path(), request(vec![change])),
                    Err(Error::Symlink { .. })
                ));
            }
        }
        for path in ["inside-link/new", "outside-link/new"] {
            assert!(matches!(
                Proposal::new(root.path(), request(vec![add(path)])),
                Err(Error::Symlink { .. })
            ));
        }
        assert_eq!(
            fs::read(root.path().join("real/file")).unwrap(),
            b"original"
        );
        assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
    }

    #[test]
    fn root_symlinks_and_symlinked_root_ancestors_are_refused() {
        let parent = TempDir::new().unwrap();
        let root = parent.path().join("real/root");
        fs::create_dir_all(&root).unwrap();
        symlink(&root, parent.path().join("root-link")).unwrap();
        symlink(
            parent.path().join("real"),
            parent.path().join("parent-link"),
        )
        .unwrap();
        for path in [
            parent.path().join("root-link"),
            parent.path().join("parent-link/root"),
        ] {
            assert!(matches!(
                Proposal::new(path, request(vec![add("new")])),
                Err(Error::Symlink { .. })
            ));
        }
        assert_eq!(fs::read_dir(root).unwrap().count(), 0);
    }

    #[test]
    fn target_symlink_inserted_after_capture_refuses_all_changes() {
        let root = TempDir::new().unwrap();
        fs::write(root.path().join("base"), b"original").unwrap();
        fs::write(root.path().join("real"), b"original").unwrap();
        let proposal = Proposal::new(
            root.path(),
            request(vec![
                add("new"),
                Change::Edit {
                    path: "base".into(),
                    content: b"proposed".to_vec(),
                },
            ]),
        )
        .unwrap();
        fs::remove_file(root.path().join("base")).unwrap();
        symlink(root.path().join("real"), root.path().join("base")).unwrap();
        let error = proposal.apply(Decision::Approve).unwrap_err();
        assert!(matches!(error.error, Error::Symlink { .. }));
        assert_eq!(error.record.decision, Status::Failed);
        assert_eq!(error.record.applied, Vec::<PathBuf>::new());
        assert!(!root.path().join("new").exists());
        assert_eq!(fs::read(root.path().join("real")).unwrap(), b"original");
    }

    #[test]
    fn parent_symlink_inserted_after_capture_refuses_all_changes() {
        let root = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        fs::create_dir(root.path().join("profiles")).unwrap();
        let proposal =
            Proposal::new(root.path(), request(vec![add("new"), add("profiles/new")])).unwrap();
        fs::remove_dir(root.path().join("profiles")).unwrap();
        symlink(outside.path(), root.path().join("profiles")).unwrap();
        let error = proposal.apply(Decision::Approve).unwrap_err();
        assert!(matches!(error.error, Error::Symlink { .. }));
        assert!(!root.path().join("new").exists());
        assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
    }

    #[test]
    fn replacing_the_protected_root_with_a_symlink_is_refused_at_approval() {
        let parent = TempDir::new().unwrap();
        let root = parent.path().join("root");
        let outside = parent.path().join("outside");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&outside).unwrap();
        let proposal = Proposal::new(&root, request(vec![add("new")])).unwrap();
        fs::remove_dir(&root).unwrap();
        symlink(&outside, &root).unwrap();
        let error = proposal.apply(Decision::Approve).unwrap_err();
        assert!(matches!(error.error, Error::Symlink { .. }));
        assert_eq!(fs::read_dir(outside).unwrap().count(), 0);
    }

    #[test]
    fn non_unicode_paths_are_refused_before_they_can_break_record_serialization() {
        let root = TempDir::new().unwrap();
        let path = std::ffi::OsString::from_vec(vec![0xff]);
        assert!(matches!(
            Proposal::new(root.path(), request(vec![add(path)])),
            Err(Error::InvalidPath { .. })
        ));
    }

    #[test]
    fn edits_preserve_permissions_and_replace_instead_of_mutating_hardlinks() {
        let root = TempDir::new().unwrap();
        let target = root.path().join("settings");
        fs::write(&target, b"original").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o640)).unwrap();
        fs::hard_link(&target, root.path().join("other-link")).unwrap();
        let proposal = Proposal::new(
            root.path(),
            request(vec![Change::Edit {
                path: "settings".into(),
                content: b"proposed".to_vec(),
            }]),
        )
        .unwrap();
        proposal.apply(Decision::Approve).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"proposed");
        assert_eq!(
            fs::read(root.path().join("other-link")).unwrap(),
            b"original"
        );
        assert_eq!(
            fs::metadata(target).unwrap().permissions().mode() & 0o777,
            0o640
        );
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
    }
}
