//! The one gh lookup: the install folders listed per system, and the first real, absolute file found.
//!
//! PREDICT: before the lookup was shared and listed per system, Windows had no winget links or Scoop shims, Linux had
//! no Homebrew-on-Linux folders, and the GitHub-reads detector searched `PATH` only (so a desktop app started with a
//! short `PATH` never found gh in Homebrew's or the user's own folder).
use super::{InstallLayout, candidates_for, locate_with};
use std::ffi::OsString;
use std::path::PathBuf;

/// A `PATH` value in this host's own separator (`:` or `;`).
fn path_of(dirs: &[&str]) -> String {
    std::env::join_paths(dirs)
        .expect("joinable")
        .into_string()
        .expect("utf-8")
}

fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
    move |key| {
        pairs
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| OsString::from(v))
    }
}

/// A Finder or Dock start: `PATH` has no Homebrew folder, the list still has it.
#[test]
fn macos_lists_homebrew_after_a_short_path() {
    let path = path_of(&["/usr/bin", "/bin", "/usr/sbin", "/sbin"]);
    let list = candidates_for(
        InstallLayout::MacOs,
        env(&[("PATH", &path), ("HOME", "/Users/t")]),
    );
    for wanted in [
        "/usr/bin/gh",
        "/opt/homebrew/bin/gh",
        "/usr/local/bin/gh",
        "/opt/local/bin/gh",
        "/Users/t/.local/bin/gh",
    ] {
        assert!(
            list.contains(&PathBuf::from(wanted)),
            "{wanted} in {list:?}"
        );
    }
    assert_eq!(list[0], PathBuf::from("/usr/bin/gh"), "PATH comes first");
    assert!(
        list.iter().position(|p| p == &PathBuf::from("/sbin/gh"))
            < list
                .iter()
                .position(|p| p == &PathBuf::from("/opt/homebrew/bin/gh")),
        "every PATH folder before the usual folders"
    );
}

#[test]
fn windows_lists_the_installer_winget_and_scoop() {
    let (pf, local, profile, scoop) = (
        r"C:\Program Files",
        r"C:\Users\t\AppData\Local",
        r"C:\Users\t",
        r"D:\scoop",
    );
    let list = candidates_for(
        InstallLayout::Windows,
        env(&[
            ("ProgramFiles", pf),
            ("LOCALAPPDATA", local),
            ("USERPROFILE", profile),
            ("SCOOP", scoop),
        ]),
    );
    let wanted = [
        PathBuf::from(pf).join("GitHub CLI").join("gh.exe"),
        PathBuf::from(local)
            .join("Programs")
            .join("GitHub CLI")
            .join("gh.exe"),
        PathBuf::from(local)
            .join("Microsoft")
            .join("WinGet")
            .join("Links")
            .join("gh.exe"),
        PathBuf::from(scoop).join("shims").join("gh.exe"),
        PathBuf::from(profile)
            .join("scoop")
            .join("shims")
            .join("gh.exe"),
    ];
    for path in &wanted {
        assert!(list.contains(path), "{} in {list:?}", path.display());
    }
    assert!(
        list.iter()
            .all(|p| p.file_name() == Some("gh.exe".as_ref())),
        "Windows looks for gh.exe only: {list:?}"
    );
}

#[test]
fn linux_lists_the_distribution_snap_homebrew_and_the_user_folder() {
    let list = candidates_for(InstallLayout::Linux, env(&[("HOME", "/home/t")]));
    for wanted in [
        "/usr/bin/gh",
        "/usr/local/bin/gh",
        "/snap/bin/gh",
        "/home/linuxbrew/.linuxbrew/bin/gh",
        "/home/t/.local/bin/gh",
        "/home/t/.linuxbrew/bin/gh",
    ] {
        assert!(
            list.contains(&PathBuf::from(wanted)),
            "{wanted} in {list:?}"
        );
    }
    assert!(
        !list.contains(&PathBuf::from("/opt/homebrew/bin/gh")),
        "macOS folders stay on macOS"
    );
}

/// A relative `PATH` entry is skipped; a real file in a usual folder is found after a `PATH` without gh.
#[test]
fn the_first_absolute_existing_file_wins() {
    let root = std::env::temp_dir().join(format!("vw-git-gh-lookup-{}", std::process::id()));
    let home = root.join("home");
    let empty = root.join("empty");
    let bin = home.join(".local").join("bin");
    std::fs::create_dir_all(&bin).expect("bin");
    std::fs::create_dir_all(&empty).expect("empty");
    std::fs::write(bin.join("gh"), b"").expect("gh");
    let path = path_of(&["relative-dir", &empty.display().to_string()]);
    let home_text = home.display().to_string();
    let found = locate_with(
        InstallLayout::Linux,
        env(&[("PATH", &path), ("HOME", &home_text)]),
    );
    assert_eq!(found, Some(bin.join("gh")));
    let on_path = format!("{}", bin.display());
    let first = locate_with(
        InstallLayout::Linux,
        env(&[("PATH", &on_path), ("HOME", "/nonexistent")]),
    );
    assert_eq!(first, Some(bin.join("gh")), "PATH entry found");
    let _ = std::fs::remove_dir_all(&root);
}

/// A fork head must keep its owner when gh runs outside the checkout.
#[cfg(unix)]
#[test]
fn fork_head_lookup_keeps_the_owner_and_checks() {
    use std::os::unix::fs::PermissionsExt;
    let root = std::env::temp_dir().join(format!("vw-git-fork-head-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let program = root.join("gh");
    std::fs::write(&program, r#"#!/bin/sh
case " $* " in
  *" --repo=github.com/upstream/project "*) ;;
  *) echo "wrong repository" >&2; exit 2 ;;
esac
for arg do selector="$arg"; done
if [ "$selector" != 'contributor:topic' ]; then
    echo 'no pull requests found' >&2
    exit 1
fi
printf '%s\n' '{"number":17,"url":"https://github.com/upstream/project/pull/17","title":"Fix","state":"OPEN","headRefName":"topic","baseRefName":"main","statusCheckRollup":[{"__typename":"CheckRun","name":"build","status":"COMPLETED","conclusion":"FAILURE"}]}'
"#).unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
    let gh = super::find(Some(&program), &root).unwrap();
    let forge = crate::forge::parse_remote("https://github.com/upstream/project").unwrap();
    let card = gh.view(&forge, "contributor:topic").unwrap().unwrap();
    assert_eq!(card.number, 17);
    assert_eq!(card.head, "topic");
    assert_eq!(card.checks.items.len(), 1);
    assert!(gh.view(&forge, "topic").unwrap().is_none());
    for head in [
        "",
        ":topic",
        "contributor:",
        "contributor:bad:name",
        "-bad:topic",
        "contributor:--help",
    ] {
        assert!(
            matches!(gh.view(&forge, head), Err(crate::GitError::Blocked(_))),
            "{head}"
        );
    }
    let _ = std::fs::remove_dir_all(root);
}
