//! Portable folder-guard regressions: Windows prefix text must survive glob extraction, and real errors refuse.
//! Depends on the parent folder module and std; called by the crate test harness on every platform.

use super::*;

#[test]
fn windows_leading_assignment_is_not_a_folder_path() -> std::io::Result<()> {
    let cwd = std::fs::canonicalize(std::env::temp_dir())?;
    // If the assignment name is treated as a path, it lands inside this guarded root.
    let layout = Layout::new(&cwd.join("R=folder/guarded"), &cwd)?;
    let permits = |_: &Path, _: bool| Ok(());
    for command in [
        "R=folder/guarded/file echo ok",
        r"R=C:\worktrees\project\ac1-git echo ok",
        r#"SRC=$(find "C:/DEVELOPER/vibe-work/project" -name favicon.jpg)"#,
    ] {
        assert_eq!(
            check_segments(
                &layout,
                crate::killparse_shell::windows_segments(command),
                None,
                &permits,
            ),
            None,
            "assignment was treated as a folder path: {command}"
        );
    }
    #[cfg(not(windows))]
    {
        // On Linux the exact Windows assignment can be a literal directory name. Guard that name to make
        // the Windows-tokenizer regression fail here too, without needing Windows' invalid-name error.
        let assignment = r"R=C:\worktrees\project\ac1-git";
        let windows_layout = Layout::new(&cwd.join(assignment), &cwd)?;
        assert_eq!(
            check_segments(
                &windows_layout,
                crate::killparse_shell::windows_segments(&format!("{assignment} echo ok")),
                None,
                &permits,
            ),
            None,
        );
    }
    assert!(
        check_segments(
            &layout,
            crate::killparse_shell::windows_segments("echo R=folder/guarded/file"),
            None,
            &permits,
        )
        .is_some(),
        "a later path-shaped argument must still be checked"
    );
    Ok(())
}

#[test]
fn command_substitutions_are_not_literal_folder_paths() {
    for command in [
        r#"SRC=$(find "C:/DEVELOPER/vibe-work/project" -name favicon.jpg)"#,
        r#"SRC="$(find '/tmp/project/assets' -name favicon.jpg)""#,
        r"SRC=`find /tmp/project/assets -name favicon.jpg`",
        r#"cp "$(find '/tmp/project/assets' -name favicon.jpg)" ./favicon.jpg"#,
    ] {
        let parsed = segments(command);
        let computed = parsed[0]
            .words
            .iter()
            .find(|word| word.expanded)
            .expect("command has a substitution");
        assert!(
            computed_shell_word(computed, &computed.text),
            "substitution was classified as a literal path: {command}"
        );
    }
    for command in [
        r#"SRC=$(find "C:/DEVELOPER/vibe-work/project" -name favicon.jpg)"#,
        r#"SRC=`find "C:\DEVELOPER\vibe-work\project" -name favicon.jpg`"#,
        r#"SRC="$(find 'C:/DEVELOPER/vibe-work/project' -name favicon.jpg)""#,
    ] {
        let parsed = crate::killparse_shell::windows_segments(command);
        assert!(
            computed_shell_word(&parsed[0].words[0], &parsed[0].words[0].text),
            "Windows substitution was classified as a literal path: {command}"
        );
    }
    let literal = &segments(r"cat './actual/file.txt'")[0].words[1];
    assert!(!computed_shell_word(literal, &literal.text));
    assert_eq!(
        shell_path_part(&literal.text, Path::new("/unused-fx9-folder")),
        Some("./actual/file.txt")
    );
}

#[cfg(windows)]
#[test]
fn windows_shell_assignment_with_find_does_not_hit_invalid_filename() -> std::io::Result<()> {
    let cwd = std::fs::canonicalize(std::env::temp_dir())?;
    let layout = Layout::new(&cwd.join("fx9-absent-vibe-home"), &cwd)?;
    let no_copy_check = |_: &Path, _: bool| -> Result<(), String> {
        panic!("a command outside Vibe home asked for a copy permit")
    };
    for command in [
        r#"SRC=$(find "C:/DEVELOPER/vibe-work/project" -name favicon.jpg)"#,
        r#"SRC=`find "C:/DEVELOPER/vibe-work/project" -name favicon.jpg`"#,
        r#"SRC="$(find 'C:/DEVELOPER/vibe-work/project' -name favicon.jpg)""#,
    ] {
        assert_eq!(
            check_command(&layout, command, None, &no_copy_check),
            None,
            "computed assignment refused: {command}"
        );
    }
    Ok(())
}

#[test]
fn windows_shell_paths_keep_backslashes_without_changing_bash_escapes() {
    use crate::killparse_shell::windows_segments;

    for path in [
        r"junction\new-file.txt",
        r"#junction\new-file.txt",
        r"junction\..\ui\src\file.txt",
        r"C:\worktrees\other\file.txt",
        r"\\?\C:\worktrees\other\file.txt",
        r"\\server\share\other\file.txt",
        r"\\?\UNC\server\share\other\file.txt",
        r"junction\../ui\src/file.txt",
    ] {
        for spelling in [path.to_owned(), format!("\"{path}\""), format!("'{path}'")] {
            let parsed = windows_segments(&format!("type {spelling}"));
            assert_eq!(parsed[0].words[1].text, path, "{spelling}");
            let parsed = windows_segments(&format!("echo x > {spelling}"));
            assert_eq!(parsed[0].redirects[0].text, path, "{spelling}");
            let parsed = windows_segments(&format!("cd {spelling} && type file.txt"));
            assert_eq!(parsed[0].words[1].text, path, "{spelling}");
            assert_eq!(parsed[1].words[1].text, "file.txt");
        }
    }
    assert_eq!(
        segments(r"cat future\ file.txt")[0].words[1].text,
        "future file.txt"
    );
    assert_eq!(
        segments(r"cat junction\new-file.txt")[0].words[1].text,
        "junctionnew-file.txt"
    );
    assert_eq!(segments(r"type #junction\file.txt")[0].words.len(), 1);
}

#[test]
fn windows_prefix_components_share_one_spelling() {
    use std::ffi::OsStr;
    use std::path::Prefix;

    for prefix in [
        Prefix::Disk(b'C'),
        Prefix::Disk(b'c'),
        Prefix::VerbatimDisk(b'C'),
        Prefix::VerbatimDisk(b'c'),
    ] {
        assert_eq!(
            normalized_prefix(prefix).as_deref(),
            Some(OsStr::new(r"\\?\C:"))
        );
    }
    for prefix in [
        Prefix::UNC(OsStr::new("SeRvEr"), OsStr::new("ShArE")),
        Prefix::VerbatimUNC(OsStr::new("server"), OsStr::new("share")),
    ] {
        assert_eq!(
            normalized_prefix(prefix).as_deref(),
            Some(OsStr::new(r"\\?\UNC\server\share"))
        );
    }
    // Other device namespaces retain their identity; they are not ordinary disk/UNC roots.
    assert_eq!(
        normalized_prefix(Prefix::Verbatim(OsStr::new("Volume{test}"))),
        None
    );
    assert_eq!(
        normalized_prefix(Prefix::DeviceNS(OsStr::new("device"))),
        None
    );
}

#[test]
fn windows_prefix_question_mark_is_not_a_glob() {
    for text in [
        r"C:\Users\Someone\Downloads\VibeWork",
        r"C:/Users/Someone\Downloads/VibeWork",
        r"\\?\C:\Users\Someone\Downloads\VibeWork",
        r"\\server\share\worktrees\repo\copy",
        r"\\?\UNC\server\share\worktrees\repo\copy",
    ] {
        assert_eq!(folder_part(text), text);
        assert_eq!(folder_part(&format!("{text}/*.rs")), format!("{text}/"));
        assert_eq!(
            folder_part(&format!("{text}/file?.rs")),
            format!("{text}/file")
        );
    }
    assert_eq!(folder_part("*.rs"), "");
    assert_eq!(folder_part("src/{a,b}.rs"), "src/");
    assert_eq!(folder_part("src/[ab].rs"), "src/");
}

#[test]
fn layout_and_path_resolution_errors_name_the_path() -> std::io::Result<()> {
    let root = std::fs::canonicalize(std::env::temp_dir())?;
    let bad = root.join("fg3-invalid\0path");
    let error = Layout::new(&root, &bad)
        .err()
        .ok_or_else(|| std::io::Error::other("invalid cwd was accepted"))?;
    assert!(error.to_string().contains("fg3-invalid"), "{error}");
    let layout = Layout::new(&root.join("fg3-absent-vibe-home"), &root)?;
    let refusal = layout.check(&bad, &root, Access::Write, &|_, _| Ok(()));
    assert!(
        refusal.as_deref().is_some_and(|text| {
            text.contains("fg3-invalid") && text.contains("could not be resolved")
        }),
        "{refusal:?}"
    );
    Ok(())
}

#[test]
fn missing_home_and_worktrees_do_not_block_an_outside_folder() -> std::io::Result<()> {
    let root = std::fs::canonicalize(std::env::temp_dir())?;
    let home = root.join("fg3-absent-vibe-home");
    let cwd = root.join("fg3-outside-project");
    let layout = Layout::new(&home, &cwd)?;
    assert!(layout.own.is_none());
    let no_copy_check = |_: &Path, _: bool| -> Result<(), String> {
        panic!("outside folder asked for a copy permit")
    };
    for access in [Access::Read, Access::Write, Access::Unknown] {
        assert!(
            layout
                .check(Path::new("future/file.txt"), &cwd, access, &no_copy_check)
                .is_none()
        );
    }
    assert!(check_command(&layout, "echo hello", None, &no_copy_check).is_none());
    Ok(())
}

#[test]
fn long_quoted_commit_body_is_not_a_folder_but_real_other_copy_is_refused() -> std::io::Result<()> {
    let id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(std::io::Error::other)?
        .as_nanos();
    let root = std::env::temp_dir().join(format!("vw-fx23-{}-{id}", std::process::id()));
    let home = root.join("vibe");
    let own = home.join("worktrees/repo/own");
    let other = home.join("worktrees/repo/other");
    std::fs::create_dir_all(own.join("docs/handbook"))?;
    std::fs::create_dir_all(&other)?;
    std::fs::write(other.join("real.txt"), b"outside copy")?;

    let result = (|| {
        let layout = Layout::new(&home, &own)?;
        let other = std::fs::canonicalize(&other)?;
        let permits = |copy: &Path, _: bool| {
            if copy == other {
                Err("another session holds this copy".to_owned())
            } else {
                Ok(())
            }
        };
        let body = format!("docs/handbook/{}\n\nMore detail", "x".repeat(320));
        assert_eq!(
            check_command(&layout, &format!("git commit -m '{body}'"), None, &permits),
            None,
            "commit message was probed as a filename"
        );
        let refusal = check_command(&layout, "cat ../other/real.txt", None, &permits);
        assert!(
            refusal
                .as_deref()
                .is_some_and(|text| text.contains("another session")),
            "real outside-copy path was allowed: {refusal:?}"
        );
        Ok(())
    })();
    std::fs::remove_dir_all(root)?;
    result
}
