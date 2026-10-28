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
                &crate::killparse_shell::windows_segments(command),
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
                &crate::killparse_shell::windows_segments(&format!("{assignment} echo ok")),
                None,
                &permits,
            ),
            None,
        );
    }
    assert!(
        check_segments(
            &layout,
            &crate::killparse_shell::windows_segments("echo R=folder/guarded/file"),
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
        shell_path_part(&literal.text, Path::new("/unused-folder")),
        Some("./actual/file.txt")
    );
}

#[cfg(windows)]
#[test]
fn windows_shell_assignment_with_find_does_not_hit_invalid_filename() -> std::io::Result<()> {
    let cwd = std::fs::canonicalize(std::env::temp_dir())?;
    let layout = Layout::new(&cwd.join("shell-find-absent-vibe-home"), &cwd)?;
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
    let bad = root.join("folders-invalid\0path");
    let error = Layout::new(&root, &bad)
        .err()
        .ok_or_else(|| std::io::Error::other("invalid cwd was accepted"))?;
    assert!(error.to_string().contains("folders-invalid"), "{error}");
    let layout = Layout::new(&root.join("folders-absent-vibe-home"), &root)?;
    let refusal = layout.check(&bad, &root, Access::Write, &|_, _| Ok(()));
    assert!(
        refusal.as_deref().is_some_and(|text| {
            text.contains("folders-invalid") && text.contains("could not resolve")
        }),
        "{refusal:?}"
    );
    Ok(())
}

#[test]
fn missing_home_and_worktrees_do_not_block_an_outside_folder() -> std::io::Result<()> {
    let root = std::fs::canonicalize(std::env::temp_dir())?;
    let home = root.join("folders-absent-vibe-home");
    let cwd = root.join("folders-outside-project");
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

#[test]
fn quoted_home_and_project_spaces_keep_full_copy_paths() -> std::io::Result<()> {
    let root = std::env::temp_dir().join(format!("vw-folder-spaces-{}", std::process::id()));
    for home_name in ["vibe-home", "vibe home"] {
        let home = root.join(home_name);
        let own = home.join("worktrees/project name-123/own");
        let other = home.join("worktrees/project name-123/other");
        std::fs::create_dir_all(&own)?;
        std::fs::create_dir_all(&other)?;
        let layout = Layout::new(&home, &own)?;
        let permits = |_: &Path, is_own: bool| {
            if is_own {
                Ok(())
            } else {
                Err("other copy busy".to_owned())
            }
        };
        for quote in ['\'', '"'] {
            for command in [
                format!("cat {quote}{}/file.txt{quote}", own.display()),
                format!(
                    "cat {quote}{}{quote}/worktrees/{quote}project name-123/own{quote}/file.txt",
                    home.display()
                ),
                format!("echo ok >{quote}{}/file.txt{quote}", own.display()),
                format!("git -C {quote}{}{quote} status --short", own.display()),
            ] {
                assert_eq!(
                    check_command(&layout, &command, None, &permits),
                    None,
                    "{command}"
                );
            }
            for path in [&other, &home] {
                let command = format!("echo ok >{quote}{}/file.txt{quote}", path.display());
                assert!(
                    check_command(&layout, &command, None, &permits).is_some(),
                    "{command}"
                );
            }
        }
    }
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn raw_mentions_keep_spaces_and_quote_concatenations_in_every_path_form() {
    for root in [
        "/tmp/vibe home",
        "/Users/test/Vibe home",
        "C:/Vibe home",
        r"C:\Vibe home",
        r"C:\ Vibe home",
        r"\\server\share\Vibe home",
        r"\\?\C:\Vibe home",
        r"\\?\UNC\server\share\Vibe home",
    ] {
        let sep = if root.contains('\\') { "\\" } else { "/" };
        let path = format!("{root}{sep}worktrees{sep}project name{sep}own{sep}file.txt");
        for quote in ['\'', '"'] {
            assert_eq!(
                raw_mentions(&format!("cat {quote}{path}{quote}"), root),
                vec![path.clone()]
            );
            assert_eq!(
                raw_mentions(
                    &format!(
                        "echo ok >{quote}{root}{quote}{sep}{quote}worktrees{sep}project name{sep}own{quote}{sep}file.txt"
                    ),
                    root
                ),
                vec![path.clone()]
            );
        }
    }
}

#[test]
fn approval_file_aliases_are_refused_before_the_file_exists() -> std::io::Result<()> {
    let data = std::env::current_dir()?.join(".scratch/approval-file-aliases");
    let layout = Layout::new(&data.join("vibe-home"), &data.join("project"))?;
    for name in [
        "plugin-approvals.json",
        "PLUGIN-APPROVALS.JSON",
        "plugin-approvals.json.",
        "plugin-approvals.json::$DATA",
    ] {
        assert!(
            layout
                .check(&data.join(name), &data, Access::Write, &|_, _| Ok(()))
                .is_some(),
            "approval-file alias must be refused: {name}"
        );
    }
    Ok(())
}

/// The Chrome Control permissions live beside the Vibe home; only the user changes them, in Settings. An agent's
/// command or tool that names that folder is refused whatever it does there (reading included: nothing in it is the
/// agent's), also when the shell only starts there.
#[test]
fn the_chrome_control_state_folder_is_refused_for_every_use() -> std::io::Result<()> {
    let data = std::fs::canonicalize(std::env::temp_dir())?
        .join(format!("vw-chrome-state-{}", std::process::id()))
        .join("chrome-control-state");
    std::fs::create_dir_all(data.join("vibe-home"))?;
    std::fs::create_dir_all(data.join("project"))?;
    let layout = Layout::new(&data.join("vibe-home"), &data.join("project"))?;
    for access in [
        Access::Read,
        Access::Write,
        Access::Unknown,
        Access::Context,
    ] {
        for name in [
            "chrome-control",
            "chrome-control/permissions.json",
            "Chrome-Control/permissions.json",
            "chrome-control./permissions.json",
        ] {
            let refusal = layout.check(&data.join(name), &data, access, &|_, _| Ok(()));
            assert!(
                refusal
                    .as_deref()
                    .is_some_and(|r| r.contains("Settings › Browser › Your Chrome")),
                "{name} with {access:?}: {refusal:?}"
            );
        }
    }
    // A neighbour with a longer name is not the folder.
    assert_eq!(
        layout.check(
            &data.join("chrome-control-notes.txt"),
            &data,
            Access::Read,
            &|_, _| Ok(())
        ),
        None
    );
    let command = format!(
        "cat {}",
        data.join("chrome-control/permissions.json").display()
    );
    assert!(check_command(&layout, &command, None, &|_, _| Ok(())).is_some());
    Ok(())
}

/// A heredoc fixture: a Vibe home under `<root>/data`, a project folder, all permitted.
fn heredoc_layout(name: &str) -> std::io::Result<(PathBuf, Layout)> {
    let root = std::fs::canonicalize(std::env::temp_dir())?
        .join(format!("vw-heredoc-{name}-{}", std::process::id()));
    let vibe_home = root.join("data").join("vibe-home");
    let cwd = root.join("project");
    std::fs::create_dir_all(&vibe_home)?;
    std::fs::create_dir_all(&cwd)?;
    Ok((root, Layout::new(&vibe_home, &cwd)?))
}

const DIVIDES: &str =
    "left = (w - side) // 2\ntop = (h - side) // 2\nratio = w / h\nprint(left, top, ratio)";

/// An inline script's body is the program's input, not shell words, so Python's
/// `// 2` and `w / h` are not the path `/` (an ancestor of Vibe Work's data folder).
#[test]
fn an_inline_script_that_divides_is_not_a_path() -> std::io::Result<()> {
    let (root, layout) = heredoc_layout("heredoc-ok")?;
    let permits = |_: &Path, _: bool| Ok(());
    let script = DIVIDES;
    for command in [
        // One bash call, `cd` first, a quoted delimiter.
        format!(
            "cd {} && python3 - <<'EOF'\n{script}\nEOF",
            layout.cwd.display()
        ),
        format!("python3 - <<'EOF'\n{script}\nEOF\necho done"),
        format!("python3 <<-\"EOF\"\n\t{script}\n\tEOF"),
    ] {
        assert_eq!(
            check_command(&layout, &command, None, &permits),
            None,
            "{command}"
        );
    }
    std::fs::remove_dir_all(&root)?;
    Ok(())
}

/// The protection stays: a body that a shell runs, a literal protected path or a quoted `/` in any
/// body, a body handed on to a shell, and every command after the body are still checked and refused.
#[test]
fn heredoc_bodies_that_really_write_there_stay_refused() -> std::io::Result<()> {
    let (root, layout) = heredoc_layout("heredoc-refused")?;
    let permits = |_: &Path, _: bool| Ok(());
    let script = DIVIDES;
    let approvals = root.join("data").join("plugin-approvals.json");
    for command in [
        // A shell runs its body: `/` there is the root folder.
        "bash <<'EOF'\nrm -rf /\nEOF".to_owned(),
        "sh -s <<EOF\nrm -rf / // 2\nEOF".to_owned(),
        // cat passes the body on to a shell.
        "cat <<'EOF' | sh\nrm -rf /\nEOF".to_owned(),
        // A here-document written into the protected file: the redirection target is a path as before.
        format!("cat > {} <<'EOF'\n{{}}\nEOF", approvals.display()),
        format!(
            "python3 - > {} <<'EOF'\nprint(7 // 2)\nEOF",
            approvals.display()
        ),
        format!("tee {} <<'EOF'\n{{}}\nEOF", approvals.display()),
        // A quoted `/` in a script is the root folder, not an operator.
        "python3 - <<'EOF'\nimport shutil\nshutil.rmtree('/')\nEOF".to_owned(),
        // The body ends at its delimiter: what follows is a command again.
        format!("python3 - <<'EOF'\n{script}\nEOF\nrm -rf /"),
        "python3 - <<-EOF\n\tx = 1 // 2\n\tEOF\nrm -rf /".to_owned(),
        // `<<<` is a here-string, not a here-document: the next line is a command.
        "cat <<< 'a // b'\nrm -rf /".to_owned(),
    ] {
        assert!(
            check_command(&layout, &command, None, &permits).is_some(),
            "must stay refused: {command}"
        );
    }
    std::fs::remove_dir_all(&root)?;
    Ok(())
}

/// Bypass attempts through other interpreters, files run later and odd delimiters: each command must be refused.
#[test]
fn heredoc_bypasses_through_interpreters_files_and_delimiters_are_refused() -> std::io::Result<()> {
    let (root, layout) = heredoc_layout("heredoc-bypass")?;
    let permits = |_: &Path, _: bool| Ok(());
    let bypasses = [
        // Perl, Ruby and JavaScript have `/`-delimited quotes and regexes: a bare `/` there may be literal.
        "perl <<'EOF'\nuse File::Path; rmtree(q / /. $ENV{HOME})\nEOF",
        "ruby <<'EOF'\nrequire 'fileutils'; FileUtils.rm_rf(%r / /.source + Dir.home)\nEOF",
        "node <<'EOF'\nrequire('fs').rmSync(/ /.source + require('os').homedir(), {recursive: true})\nEOF",
        // Python: a `/` beside a string literal may join a path (`Path.home() / '.local'`).
        "python3 - <<'EOF'\nimport pathlib, shutil\nshutil.rmtree(pathlib.Path.home() / '.local' / 'share')\nEOF",
        // A cat/tee body written to a file that a later command runs.
        "cat > run.sh <<'EOF'\nrm -rf /\nEOF\nsh run.sh",
        "tee run.sh <<'EOF' >/dev/null\nrm -rf /\nEOF\nbash run.sh",
        // The interpreter's output handed on to a shell, also through a line continuation.
        "python3 - <<'EOF' | sh\nprint('rm -rf ' + chr(47))\nrm -rf /\nEOF",
        "python3 - <<'EOF' \\\n| sh\nrm -rf /\nEOF",
        // Delimiters bash reads differently (`$'..'`, `$".."`, expansions) and a body with no closing line.
        "python3 - <<$'EOF'\nx = 1\nEOF\nrm -rf /\n$EOF",
        "python3 - <<$\"EOF\"\nx = 1\nEOF\nrm -rf /\n$EOF",
        "python3 - <<$D\nx = 1\n$D\nrm -rf /",
        "python3 - <<'EOF'\nrm -rf /",
    ]
    .into_iter()
    .filter(|command| check_command(&layout, command, None, &permits).is_none())
    .collect::<Vec<_>>();
    std::fs::remove_dir_all(&root)?;
    assert!(bypasses.is_empty(), "must be refused: {bypasses:#?}");
    Ok(())
}

/// Bypass attempts through command grouping, quoting and other invocation forms: each command must be refused (the
/// exemption must not apply, so the `/` is checked as before).
#[test]
fn heredoc_bypasses_through_grouping_quoting_and_forms_are_refused() -> std::io::Result<()> {
    let (root, layout) = heredoc_layout("heredoc-bypass2")?;
    let permits = |_: &Path, _: bool| Ok(());
    let bypasses = [
        // A here-document belongs to the command being read at its `<<`, not to the next one kept.
        "( bash ) <<'EOF'; python3 -\nrm -rf /\nEOF",
        "{ bash; } <<'EOF'; python3 -\nrm -rf /\nEOF",
        "bash <<'EOF' && python3 -\nrm -rf /\nEOF",
        // Quoting the shell reads differently from Python (a multi-line string, a quote in a comment).
        "python3 - <<'EOF'\nimport shutil\np = \'\'\'\n\'\n/\n\'\'\'\nshutil.rmtree(p.split()[-1])\nEOF",
        "python3 - <<'EOF'\nimport shutil  # it's the root\nx = 1 / 2\nshutil.rmtree(chr(47))\nEOF",
        "python3 - <<'EOF'\ns = \"\\\" / \\\"\"\nprint(1 / 2)\nEOF",
        // Output to a file a later command runs, a process substitution, other forms than `python3 -`.
        "python3 - > run.sh <<'EOF'\nprint(1 / 2)\nEOF\nsh run.sh",
        "bash <(python3 - <<'EOF'\nprint(1 / 2)\nEOF\n)",
        "python3 - x <<'EOF'\nprint(1 / 2)\nEOF",
        "sudo python3 - <<'EOF'\nprint(1 / 2)\nEOF",
        "python3 - <<EOF\nprint(1 / 2)\nEOF",
    ]
    .into_iter()
    .filter(|command| check_command(&layout, command, None, &permits).is_none())
    .collect::<Vec<_>>();
    std::fs::remove_dir_all(&root)?;
    assert!(bypasses.is_empty(), "must be refused: {bypasses:#?}");
    Ok(())
}
