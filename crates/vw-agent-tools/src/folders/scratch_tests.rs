//! Temporary chats can read Vibe skills and use their own scratch folder, while other app data stays private.
//! Exercises the folder guard with real folders; native case and Windows-prefix checks run on those platforms.
use super::*;

/// A folder-less temporary chat works in `<app data>/temporary-folders/chat-<hex>`, beside the session data: the
/// guard lets it work there in each system's app data layout, and still refuses the session data, including the
/// place earlier test builds made those folders (`<app data>/sessions/temporary-folders`).
#[test]
fn a_temporary_chats_scratch_folder_beside_the_session_data_is_usable_in_every_app_data_layout()
-> std::io::Result<()> {
    let base = std::fs::canonicalize(std::env::temp_dir())?
        .join(format!("scratch-guard-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let permits = |_: &Path, _: bool| Ok(());
    for layout_root in [
        "AppData/Roaming/app.vibework.desktop",
        "Library/Application Support/app.vibework.desktop",
        ".local/share/app.vibework.desktop",
    ] {
        let data = base.join(layout_root);
        let own = data.join("temporary-folders").join("chat-7ef176a0a74816fb");
        let old = data
            .join("sessions")
            .join("temporary-folders")
            .join("chat-c873b87c9ce80277");
        let sibling = data.join("temporary-folders/chat-0011223344556677");
        for dir in [&data.join("vibe-home"), &own, &old, &sibling] {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(data.join("keys.json"), b"private")?;
        let layout = Layout::new(&data.join("vibe-home"), &own)?;
        assert_scratch_access(&layout, &own);
        for cwd in [
            data.clone(),
            data.join("temporary-folders"),
            data.join("sessions"),
            data.join("temporary-folders/chat-not-a-scratch-folder"),
        ] {
            let invalid = Layout::new(&data.join("vibe-home"), &cwd)?;
            for access in [
                Access::Read,
                Access::Write,
                Access::Unknown,
                Access::Context,
            ] {
                assert!(
                    invalid.check(&cwd, &cwd, access, &permits).is_some(),
                    "invalid exception: {cwd:?}"
                );
            }
        }
        #[cfg(any(windows, target_os = "macos"))]
        assert_scratch_spellings(&layout, &data, &own);
        for path in [old.join("x"), data.join("sessions/temporary.json")] {
            let refused = layout.check(&path, &own, Access::Write, &permits);
            assert!(
                refused
                    .as_deref()
                    .is_some_and(|r| r.contains("own session data")),
                "{path:?}: {refused:?}"
            );
        }
    }
    std::fs::remove_dir_all(&base)?;
    Ok(())
}

fn assert_scratch_access(layout: &Layout, own: &Path) {
    let permits = |_: &Path, _: bool| Ok(());
    for access in [
        Access::Read,
        Access::Write,
        Access::Unknown,
        Access::Context,
    ] {
        for path in [
            PathBuf::from("../chat-0011223344556677/notes.txt"),
            PathBuf::from("../../keys.json"),
            PathBuf::from("../.."),
            PathBuf::from(".."),
            PathBuf::from("../../sessions/temporary.json"),
            PathBuf::from("../../vibe-home/worktrees/project/copy/file.txt"),
            own.with_file_name("chat-7ef176a0a74816fb-extra"),
        ] {
            assert!(
                layout.check(&path, own, access, &permits).is_some(),
                "app data escaped: {access:?} {path:?} under {own:?}"
            );
        }
        for path in ["sub/../notes.txt", "../chat-7ef176a0a74816fb/notes.txt"] {
            assert_eq!(layout.check(Path::new(path), own, access, &permits), None);
        }
    }
    for access in [
        Access::Read,
        Access::Write,
        Access::Unknown,
        Access::Context,
    ] {
        for path in [
            own.to_path_buf(),
            own.join("notes.txt"),
            own.join("sub/dir"),
        ] {
            assert_eq!(
                layout.check(&path, own, access, &permits),
                None,
                "{access:?} {path:?}"
            );
        }
        assert_eq!(
            layout.check(Path::new("notes.txt"), own, access, &permits),
            None
        );
    }
}

#[cfg(any(windows, target_os = "macos"))]
fn assert_scratch_spellings(layout: &Layout, data: &Path, own: &Path) {
    let permits = |_: &Path, _: bool| Ok(());
    for access in [Access::Read, Access::Write, Access::Unknown] {
        let mixed = PathBuf::from(own.to_string_lossy().to_uppercase());
        assert_eq!(
            layout.check(&mixed.join("sub/../notes.txt"), own, access, &permits),
            None
        );
        assert!(
            layout
                .check(&mixed.join("../../keys.json"), own, access, &permits)
                .is_some()
        );
        assert!(
            layout
                .check(
                    &mixed.join("../chat-0011223344556677"),
                    own,
                    access,
                    &permits
                )
                .is_some()
        );
    }
    #[cfg(windows)]
    {
        let verbatim = resolve(own).unwrap();
        for spelling in [
            verbatim.clone(),
            PathBuf::from(
                verbatim
                    .to_string_lossy()
                    .replace('\\', "/")
                    .replacen("//?/", r"\\?\", 1),
            ),
        ] {
            let alternate = Layout::new(&data.join("vibe-home"), &spelling).unwrap();
            for access in [Access::Read, Access::Write, Access::Unknown] {
                assert_eq!(
                    alternate.check(Path::new("sub/../notes.txt"), &spelling, access, &permits),
                    None
                );
                assert!(
                    alternate
                        .check(
                            Path::new("../chat-0011223344556677"),
                            &spelling,
                            access,
                            &permits
                        )
                        .is_some()
                );
                assert!(
                    alternate
                        .check(Path::new("../../keys.json"), &spelling, access, &permits)
                        .is_some()
                );
            }
        }
    }
    #[cfg(not(windows))]
    let _ = data;
}

#[test]
fn normal_and_temporary_chats_can_read_skills_but_cannot_write_the_vibe_home() {
    let root = std::env::temp_dir().join(format!("skill-folder-guard-{}", std::process::id()));
    let data = root.join("data");
    let home = data.join("vibe-home");
    let skill = home.join("logs/session/plugins/packages/example/skills/example");
    let scratch = data.join("temporary-folders/chat-0011223344556677");
    let copy = home.join("worktrees/project/own");
    for folder in [&skill, &scratch, &copy] {
        std::fs::create_dir_all(folder).unwrap();
    }
    for file in [skill.join("SKILL.md"), skill.join("references/guide.md")] {
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, "skill reference").unwrap();
    }
    let failures: Vec<_> = [&copy, &scratch]
        .into_iter()
        .flat_map(|cwd| {
            let layout = Layout::new(&home, cwd).unwrap();
            [
                skill.join("SKILL.md"),
                skill.join("references/guide.md"),
                home.join("config.toml"),
            ]
            .into_iter()
            .filter_map(move |path| {
                for access in [Access::Write, Access::Unknown] {
                    assert!(layout.check(&path, cwd, access, &|_, _| Ok(())).is_some());
                }
                check_tool_input(
                    &layout,
                    &serde_json::json!({"file_path": path}),
                    Access::Read,
                    &|_, _| panic!("a skill read must not request a copy permit"),
                )
            })
        })
        .collect();
    std::fs::remove_dir_all(root).unwrap();
    assert!(failures.is_empty(), "skill reads refused: {failures:#?}");
}

#[test]
fn temporary_chats_have_no_copy_permit_and_cannot_read_private_app_data() {
    let root = std::env::temp_dir().join(format!("temporary-copy-guard-{}", std::process::id()));
    let home = root.join("vibe-home");
    let scratch = root.join("temporary-folders/chat-0011223344556677");
    let copy = home.join("worktrees/project/other");
    for folder in [&scratch, &copy] {
        std::fs::create_dir_all(folder).unwrap();
    }
    let layout = Layout::new(&home, &scratch).unwrap();
    for access in [
        Access::Read,
        Access::Write,
        Access::Unknown,
        Access::Context,
    ] {
        let refusal = layout
            .check(&copy.join("file.txt"), &scratch, access, &|_, _| Ok(()))
            .unwrap();
        assert!(
            refusal.contains("session copy"),
            "copy check skipped: {refusal}"
        );
        for path in [
            "../../keys.json",
            "../chat-8899aabbccddeeff",
            "../../sessions/record.json",
            "../../plugin-approvals.json",
            "../../chrome-control/state.json",
            "..",
        ] {
            assert!(
                layout
                    .check(Path::new(path), &scratch, access, &|_, _| Ok(()))
                    .is_some()
            );
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}
