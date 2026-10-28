//! Linux entries: what is written, that removal deletes exactly those files, and the all-or-nothing write.

use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;

use super::{MARKER, entry_paths, install_in, installed_in, remove_in};

const EXE: &str = "/opt/Vibe Work/vibe-work";

fn files_under(root: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                found.push(path.strip_prefix(root).unwrap().display().to_string());
            }
        }
    }
    found.sort();
    found
}

#[test]
fn switching_on_writes_three_entries_that_run_attach() {
    let home = tempfile::tempdir().unwrap();
    assert!(!installed_in(home.path()));
    install_in(home.path(), Path::new(EXE)).unwrap();
    assert!(installed_in(home.path()));
    assert_eq!(
        files_under(home.path()),
        vec![
            "applications/vibe-work-send-to.desktop",
            "kio/servicemenus/vibe-work-send-to.desktop",
            "nautilus/scripts/Send to Vibe Work",
        ]
    );
    let [desktop, nautilus, dolphin] = entry_paths(home.path());
    let desktop = std::fs::read_to_string(desktop).unwrap();
    assert!(desktop.contains("Exec=\"/opt/Vibe Work/vibe-work\" --attach %F\n"));
    assert!(desktop.contains("Name=Send to Vibe Work\n"));
    assert!(desktop.contains("NoDisplay=true\n"));
    let menu = std::fs::read_to_string(&dolphin).unwrap();
    assert!(menu.contains("Exec=\"/opt/Vibe Work/vibe-work\" --attach %F\n"));
    assert!(menu.contains("X-KDE-ServiceTypes=KonqPopupMenu/Plugin\n"));
    let script = std::fs::read_to_string(&nautilus).unwrap();
    assert_eq!(
        script,
        format!("#!/bin/sh\n# {MARKER}\nexec '/opt/Vibe Work/vibe-work' --attach \"$@\"\n")
    );
    for executable in [&nautilus, &dolphin] {
        let mode = std::fs::metadata(executable).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700, "{}", executable.display());
    }
    // Writing again (a repair) replaces our own files.
    install_in(home.path(), Path::new(EXE)).unwrap();
}

#[test]
fn switching_off_removes_exactly_what_was_written() {
    let home = tempfile::tempdir().unwrap();
    let neighbour = home.path().join("nautilus/scripts/My own script");
    std::fs::create_dir_all(neighbour.parent().unwrap()).unwrap();
    std::fs::write(&neighbour, "#!/bin/sh\n").unwrap();
    install_in(home.path(), Path::new(EXE)).unwrap();
    remove_in(home.path()).unwrap();
    assert!(!installed_in(home.path()));
    assert_eq!(
        files_under(home.path()),
        vec!["nautilus/scripts/My own script"]
    );
    // Removing again is fine (the uninstall and the switch may both run).
    remove_in(home.path()).unwrap();
}

#[test]
fn a_file_vibe_work_did_not_write_is_never_replaced_or_removed() {
    let home = tempfile::tempdir().unwrap();
    let [desktop, ..] = entry_paths(home.path());
    std::fs::create_dir_all(desktop.parent().unwrap()).unwrap();
    std::fs::write(&desktop, "[Desktop Entry]\nName=Mine\n").unwrap();
    let error = install_in(home.path(), Path::new(EXE)).unwrap_err();
    assert!(
        error.to_string().contains("was not written by Vibe Work"),
        "{error}"
    );
    // All or nothing: the entries written before the failure are gone again.
    assert_eq!(
        files_under(home.path()),
        vec!["applications/vibe-work-send-to.desktop"]
    );
    remove_in(home.path()).unwrap();
    assert_eq!(
        std::fs::read_to_string(&desktop).unwrap(),
        "[Desktop Entry]\nName=Mine\n"
    );
}

#[test]
fn quoting_survives_unusual_program_paths() {
    let home = tempfile::tempdir().unwrap();
    install_in(home.path(), Path::new("/home/a'b/100% \"x\"/$app")).unwrap();
    let [desktop, nautilus, _] = entry_paths(home.path());
    assert!(
        std::fs::read_to_string(desktop)
            .unwrap()
            .contains(r#"Exec="/home/a'b/100%% \"x\"/\$app" --attach %F"#)
    );
    assert!(
        std::fs::read_to_string(nautilus)
            .unwrap()
            .contains(r#"exec '/home/a'\''b/100% "x"/$app' --attach "$@""#)
    );
}
