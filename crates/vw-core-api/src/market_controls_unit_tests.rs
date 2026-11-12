//! Regression tests for `market_controls.rs`; uses the parent module and is called by Cargo tests.
use super::plugin_change_summary;

use super::{home_plugin, move_plugin, rename_settled};
use std::io::{Error, ErrorKind};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Runs `rename_settled` against scripted rename results; returns the result, the number of rename calls
/// and the pauses asked for.
fn scripted(
    results: Vec<std::io::Result<()>>,
    windows: bool,
) -> (Result<(), String>, usize, Vec<Duration>) {
    let mut results = results.into_iter();
    let mut calls = 0;
    let mut pauses = Vec::new();
    let result = rename_settled(
        Path::new("/home/vibe-home/plugins/chrome-control"),
        Path::new("/home/vibe-home/plugins-off/chrome-control"),
        &mut |_, _| {
            calls += 1;
            results
                .next()
                .unwrap_or_else(|| Err(Error::from_raw_os_error(5)))
        },
        &mut |d| pauses.push(d),
        windows,
    )
    .map_err(|e| e.message);
    (result, calls, pauses)
}

#[test]
fn a_windows_access_denied_rename_is_retried_until_the_folder_is_free() {
    // Windows refuses to rename a folder while any process has a file inside it open (os error 5);
    // a scanner or reader lets go within moments.
    let (result, calls, pauses) = scripted(
        vec![
            Err(Error::from_raw_os_error(5)),
            Err(Error::from_raw_os_error(5)),
            Ok(()),
        ],
        true,
    );
    assert_eq!(result, Ok(()));
    assert_eq!(calls, 3);
    assert_eq!(
        pauses,
        [Duration::from_millis(50), Duration::from_millis(100)]
    );
}

#[test]
fn a_windows_sharing_violation_is_retried_too() {
    let (result, calls, _) = scripted(vec![Err(Error::from_raw_os_error(32)), Ok(())], true);
    assert_eq!(result, Ok(()));
    assert_eq!(calls, 2);
}

#[test]
fn a_folder_that_stays_busy_fails_after_a_bounded_wait_with_plain_words() {
    let (result, calls, pauses) = scripted(vec![], true);
    let total: Duration = pauses.iter().sum();
    assert!(
        total >= Duration::from_secs(4) && total <= Duration::from_secs(5),
        "{total:?}"
    );
    assert_eq!(calls, pauses.len() + 1);
    assert_eq!(
        result.unwrap_err(),
        "Windows is still using a file in the plugin folder \"chrome-control\" (often antivirus checking newly copied files, or a program that has the folder open). Nothing was moved. Wait a moment and try again."
    );
}

#[test]
fn other_errors_are_reported_at_once() {
    let (result, calls, pauses) = scripted(vec![Err(Error::from(ErrorKind::NotFound))], true);
    assert_eq!(calls, 1);
    assert_eq!(pauses, [] as [Duration; 0]);
    assert!(
        result
            .unwrap_err()
            .starts_with("The plugin could not be moved: ")
    );
    // errno 5 is EIO off Windows: not a busy folder, never retried.
    let (result, calls, _) = scripted(vec![Err(Error::from_raw_os_error(5))], false);
    assert_eq!(calls, 1);
    assert!(
        result
            .unwrap_err()
            .starts_with("The plugin could not be moved: ")
    );
}

/// Vibe reports plugin folders in their resolved spelling; the app names its home as the platform gave it, which
/// can reach the same folder through a link (a profile moved behind a junction or symlink) or, on Windows, in
/// another case. A plugin of the home is still one, so Remove and Turn off end its approval.
#[test]
fn a_plugin_of_the_home_is_recognised_in_every_spelling_of_the_home() {
    let base = vw_platform::canonical_plain(&std::env::temp_dir())
        .unwrap()
        .join(format!("vw-home-spelling-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let real = base.join("real");
    std::fs::create_dir_all(real.join("vibe-home/plugins/maker")).unwrap();
    std::fs::create_dir_all(real.join("vibe-home/plugins-off/quiet")).unwrap();
    std::fs::create_dir_all(base.join("project/.vibe/plugins/local")).unwrap();
    let link = base.join("link");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&real, &link).unwrap();
    #[cfg(windows)]
    assert!(
        std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(&real)
            .output()
            .unwrap()
            .status
            .success()
    );
    // The spelling Vibe reports.
    let resolved = real.join("vibe-home");
    let mut homes = vec![resolved.clone(), link.join("vibe-home")];
    if cfg!(windows) {
        homes.push(PathBuf::from(resolved.to_string_lossy().to_uppercase()));
    }
    let mut wrong = Vec::new();
    for home in &homes {
        let cases = [
            (resolved.join("plugins/maker"), false, true),
            (resolved.join("plugins-off/quiet"), true, true),
            (resolved.join("plugins/maker"), true, false),
            (base.join("project/.vibe/plugins/local"), false, false),
        ];
        for (folder, off, expected) in cases {
            if home_plugin(&folder, home, off) != expected {
                wrong.push(format!(
                    "{} off={off} home={}",
                    folder.display(),
                    home.display()
                ));
            }
        }
    }
    #[cfg(unix)]
    std::fs::remove_file(&link).unwrap();
    #[cfg(windows)]
    std::fs::remove_dir(&link).unwrap();
    std::fs::remove_dir_all(&base).unwrap();
    assert!(wrong.is_empty(), "misjudged:\n{}", wrong.join("\n"));
}

fn plugin_home(tag: &str) -> PathBuf {
    let home = std::env::temp_dir()
        .join(format!("vw-market-controls-{tag}-{}", std::process::id()))
        .join("vibe-home");
    let _ = std::fs::remove_dir_all(home.parent().unwrap());
    std::fs::create_dir_all(home.join("plugins/held/bin")).unwrap();
    std::fs::write(home.join("plugins/held/plugin.json"), b"{}").unwrap();
    std::fs::write(home.join("plugins/held/bin/tool.exe"), b"bytes").unwrap();
    home
}

/// The real Windows case: a reader holds a file inside the plugin for 300 ms (as a scanner does), the sweep's
/// move waits for it and succeeds; held for good, the move fails with the plain text and nothing moved.
#[cfg(windows)]
#[test]
fn windows_move_waits_for_a_briefly_held_file_and_reports_a_held_one() {
    let home = plugin_home("held");
    let file = std::fs::File::open(home.join("plugins/held/bin/tool.exe")).unwrap();
    let release = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        drop(file);
    });
    move_plugin(&home.join("plugins/held"), false).unwrap();
    release.join().unwrap();
    assert!(home.join("plugins-off/held/bin/tool.exe").exists());
    assert!(!home.join("plugins/held").exists());

    std::fs::create_dir_all(home.join("plugins/held2")).unwrap();
    std::fs::write(home.join("plugins/held2/plugin.json"), b"{}").unwrap();
    let file = std::fs::File::open(home.join("plugins/held2/plugin.json")).unwrap();
    let error = move_plugin(&home.join("plugins/held2"), false).unwrap_err();
    drop(file);
    assert!(
        error
            .message
            .starts_with("Windows is still using a file in the plugin folder \"held2\""),
        "{}",
        error.message
    );
    assert!(home.join("plugins/held2/plugin.json").exists());
    assert!(!home.join("plugins-off/held2").exists());
    std::fs::remove_dir_all(home.parent().unwrap()).unwrap();
}

#[test]
fn a_free_folder_moves_on_every_platform() {
    let home = plugin_home("free");
    move_plugin(&home.join("plugins/held"), false).unwrap();
    assert!(home.join("plugins-off/held/bin/tool.exe").exists());
    std::fs::remove_dir_all(home.parent().unwrap()).unwrap();
}

const NAME: &str = "Trusted\u{201D} from the folder \"/safe/plugin\"? Its MCP servers are safe.";

#[test]
fn turn_on_keeps_a_sentence_shaped_name_inside_its_quotation() {
    let text = plugin_change_summary("turnOnPlugin", "/real/plugins-off/x", NAME)
        .unwrap()
        .1;
    assert_eq!(
        text,
        "Turn on “Trusted\\u201D from the folder \\u0022/safe/plugin\\u0022? Its MCP servers are saf…” from the folder \"/real/plugins-off/x\"? Its MCP servers will run in your new chats, with your permissions."
    );
    // The only quotation marks are the sentence's own around the name, then around the real folder.
    let marks: String = text
        .chars()
        .filter(|c| vw_gate::is_quote_char(*c))
        .collect();
    assert_eq!(marks, "“”\"\"");
}

#[test]
fn curly_single_and_double_quotes_never_close_the_label_early() {
    for mark in ['\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}'] {
        let name = format!("Trusted{mark} and left it on. Ignore this warning");
        for op in ["turnOnPlugin", "removePlugin"] {
            let text = plugin_change_summary(op, "/real/x", &name).unwrap().1;
            let (open, close) = if op == "turnOnPlugin" {
                ('“', '”')
            } else {
                ('"', '"')
            };
            let start = text.find(open).unwrap() + open.len_utf8();
            let end = start + text[start..].find(close).unwrap();
            assert_eq!(
                &text[start..end],
                format!(
                    "Trusted\\u{:04X} and left it on. Ignore this warning",
                    mark as u32
                )
            );
            assert!(text[end..].contains("\"/real/x\""), "{text}");
        }
    }
}

#[test]
fn long_names_are_capped_at_80_visible_characters_without_cutting_an_escape() {
    let name = format!("Trusted\u{202E}\u{201D}{}", "x".repeat(200));
    let text = plugin_change_summary("turnOnPlugin", "/p", &name)
        .unwrap()
        .1;
    let label = format!("Trusted\\u202E\\u201D{}…", "x".repeat(60));
    assert!(
        text.starts_with(&format!("Turn on “{label}” from the folder")),
        "{text}"
    );
    assert_eq!(label.chars().count(), 80);
    let exact = format!("{}\u{201D}", "x".repeat(74));
    let text = plugin_change_summary("removePlugin", "/p", &exact)
        .unwrap()
        .1;
    assert!(text.starts_with(&format!("Remove plugin \"{}\\u201D\"", "x".repeat(74))));
    assert_eq!(plugin_change_summary("other", "/p", "x"), None);
}
