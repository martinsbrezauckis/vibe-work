//! Unit tests for `command_resolve` with fake environments for Windows, macOS and Linux (no real file system).
use std::collections::{HashMap, HashSet};

use super::*;

#[derive(Default)]
struct Fake {
    vars: HashMap<&'static str, &'static str>,
    files: HashSet<&'static str>,
    dirs: HashSet<&'static str>,
    plain: HashSet<&'static str>,
    login: Option<&'static str>,
    interactive: Option<&'static str>,
}

impl Probe for Fake {
    fn var(&self, name: &str) -> Option<String> {
        self.vars.get(name).map(|v| (*v).to_owned())
    }
    fn kind(&self, path: &str) -> Option<EntryKind> {
        if self.files.contains(path) || self.plain.contains(path) {
            Some(EntryKind::File)
        } else if self.dirs.contains(path) {
            Some(EntryKind::Dir)
        } else {
            None
        }
    }
    fn is_executable(&self, path: &str) -> bool {
        self.files.contains(path)
    }
    fn login_path(&self, interactive: bool) -> Option<String> {
        if interactive {
            self.interactive.map(str::to_owned)
        } else {
            self.login.map(str::to_owned)
        }
    }
}

#[test]
fn linux_finds_a_program_on_path_and_skips_files_without_the_run_bit() {
    let fake = Fake {
        vars: [("PATH", "/opt/a:/usr/local/bin:/usr/bin")].into(),
        files: ["/usr/bin/uvx"].into(),
        plain: ["/opt/a/uvx"].into(),
        ..Fake::default()
    };
    assert_eq!(
        resolve(OsKind::Linux, "uvx", None, &fake).unwrap(),
        "/usr/bin/uvx"
    );
}

#[test]
fn linux_not_found_directory_and_not_runnable_each_have_their_own_reason() {
    let fake = Fake {
        vars: [("PATH", "/usr/bin"), ("HOME", "/home/u")].into(),
        dirs: ["/opt/tool"].into(),
        plain: ["/home/u/bin/note.txt"].into(),
        ..Fake::default()
    };
    let missing = resolve(OsKind::Linux, "nope", None, &fake).unwrap_err();
    assert!(
        matches!(missing, ResolveError::NotFound { .. }),
        "{missing:?}"
    );
    let text = missing.to_string().to_ascii_lowercase();
    assert!(
        text.contains("not found") && text.contains("full path"),
        "{text}"
    );
    let dir = resolve(OsKind::Linux, "/opt/tool", None, &fake).unwrap_err();
    assert!(matches!(dir, ResolveError::IsFolder { .. }), "{dir:?}");
    let plain = resolve(OsKind::Linux, "~/bin/note.txt", None, &fake).unwrap_err();
    assert!(
        matches!(plain, ResolveError::NotRunnable { .. }),
        "{plain:?}"
    );
}

#[test]
fn linux_paths_are_made_absolute_without_following_links_and_relative_needs_a_folder() {
    let fake = Fake {
        vars: [("PATH", "/usr/bin"), ("HOME", "/home/u")].into(),
        files: ["/home/u/proj/.venv/bin/python", "/home/u/tool"].into(),
        ..Fake::default()
    };
    assert_eq!(
        resolve(
            OsKind::Linux,
            "./.venv/bin/python",
            Some("/home/u/proj"),
            &fake
        )
        .unwrap(),
        "/home/u/proj/.venv/bin/python"
    );
    assert_eq!(
        resolve(OsKind::Linux, "~/tool", None, &fake).unwrap(),
        "/home/u/tool"
    );
    assert_eq!(
        resolve(OsKind::Linux, "/home/u/proj/../tool", None, &fake).unwrap(),
        "/home/u/tool"
    );
    assert!(matches!(
        resolve(OsKind::Linux, "./x", None, &fake).unwrap_err(),
        ResolveError::RelativeNeedsFolder
    ));
}

#[test]
fn empty_and_disguised_input_is_refused() {
    let fake = Fake::default();
    for bad in ["", "   ", "a\0b", "a\nb", "tool\u{202e}exe"] {
        let error = resolve(OsKind::Linux, bad, None, &fake).unwrap_err();
        assert!(
            matches!(error, ResolveError::Empty | ResolveError::BadCharacters),
            "{bad:?}: {error:?}"
        );
    }
}

#[test]
fn macos_gui_path_misses_homebrew_until_the_login_shell_path_is_merged() {
    let fake = Fake {
        vars: [("PATH", "/usr/bin:/bin")].into(),
        files: ["/opt/homebrew/bin/npx", "/usr/bin/true"].into(),
        login: Some("/opt/homebrew/bin:/usr/bin:/bin"),
        ..Fake::default()
    };
    assert_eq!(
        resolve(OsKind::MacOs, "npx", None, &fake).unwrap(),
        "/opt/homebrew/bin/npx"
    );
    assert_eq!(
        resolve(OsKind::MacOs, "true", None, &fake).unwrap(),
        "/usr/bin/true"
    );
    // Linux does not ask a shell.
    assert!(resolve(OsKind::Linux, "npx", None, &fake).is_err());
}

#[test]
fn macos_asks_the_interactive_shell_only_when_the_login_path_has_no_such_program() {
    let fake = Fake {
        vars: [("PATH", "/usr/bin")].into(),
        files: ["/Users/u/.nvm/bin/node"].into(),
        login: Some("/usr/bin"),
        interactive: Some("/Users/u/.nvm/bin:/usr/bin"),
        ..Fake::default()
    };
    assert_eq!(
        resolve(OsKind::MacOs, "node", None, &fake).unwrap(),
        "/Users/u/.nvm/bin/node"
    );
}

fn windows() -> Fake {
    Fake {
        vars: [
            (
                "PATH",
                r"C:\Windows\System32;C:\Program Files\nodejs;C:\Tools",
            ),
            ("PATHEXT", ".COM;.EXE;.BAT;.CMD"),
        ]
        .into(),
        files: [
            r"C:\Program Files\nodejs\npx.cmd",
            r"C:\Program Files\nodejs\node.exe",
            r"C:\Tools\mcp-fixture.exe",
            r"C:\Tools\run.bat",
        ]
        .into(),
        ..Fake::default()
    }
}

#[test]
fn windows_resolves_bare_names_through_pathext_and_keeps_explicit_extensions() {
    let fake = windows();
    assert_eq!(
        resolve(OsKind::Windows, "node", None, &fake).unwrap(),
        r"C:\Program Files\nodejs\node.exe"
    );
    assert_eq!(
        resolve(OsKind::Windows, "npx", None, &fake).unwrap(),
        r"C:\Program Files\nodejs\npx.cmd"
    );
    assert_eq!(
        resolve(OsKind::Windows, "mcp-fixture.exe", None, &fake).unwrap(),
        r"C:\Tools\mcp-fixture.exe"
    );
    assert_eq!(
        resolve(OsKind::Windows, r"C:\Tools\mcp-fixture", None, &fake).unwrap(),
        r"C:\Tools\mcp-fixture.exe"
    );
    assert_eq!(
        resolve(OsKind::Windows, "C:/Tools/mcp-fixture.exe", None, &fake).unwrap(),
        r"C:\Tools\mcp-fixture.exe"
    );
    assert!(matches!(
        resolve(OsKind::Windows, "ghost", None, &fake).unwrap_err(),
        ResolveError::NotFound { .. }
    ));
}

#[test]
fn windows_scripts_start_only_with_plain_arguments_and_exe_always() {
    let exe = r"C:\Tools\mcp-fixture.exe";
    assert!(check_launch(OsKind::Windows, exe, &["--a b".to_owned()]).is_ok());
    let cmd = r"C:\Program Files\nodejs\npx.cmd";
    let plain = [
        "-y".to_owned(),
        "@scope/pkg@1.2.3".to_owned(),
        "C:\\data".to_owned(),
    ];
    assert!(check_launch(OsKind::Windows, cmd, &plain).is_ok());
    for bad in ["a&calc", "a b", "%PATH%", "x\"y", "a|b", "a^b", "a>b", "!x"] {
        let error = check_launch(OsKind::Windows, cmd, &[bad.to_owned()]).unwrap_err();
        assert!(
            matches!(error, ResolveError::ScriptArguments { .. }),
            "{bad}: {error:?}"
        );
        let text = error.to_string();
        assert!(text.contains("npx.cmd") && text.contains(".exe"), "{text}");
    }
    let ps1 = check_launch(OsKind::Windows, r"C:\Tools\x.ps1", &[]).unwrap_err();
    assert!(matches!(ps1, ResolveError::NotAProgram { .. }), "{ps1:?}");
    // Other systems never apply the Windows script rule.
    assert!(check_launch(OsKind::Linux, "/usr/bin/npx", &["a&b".to_owned()]).is_ok());
}
