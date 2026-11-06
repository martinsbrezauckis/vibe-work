//! Instruction writers share replacement guarantees across Settings and startup.
//! Uses isolated files and the actual writers; run by the core crate unit tests.

use super::write_atomic;
use std::fs;

#[test]
fn editor_replacement_ignores_stale_process_temp_names() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("AGENTS.md");
    let stale = home
        .path()
        .join(format!(".AGENTS.md.{}.tmp", std::process::id()));
    fs::write(&stale, "left by an interrupted save").unwrap();
    fs::write(&path, "before").unwrap();
    write_atomic(&path, "after").unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "after");
    assert_eq!(
        fs::read_to_string(&stale).unwrap(),
        "left by an interrupted save"
    );
    assert_eq!(fs::read_dir(home.path()).unwrap().count(), 2);
}

#[test]
fn startup_replacement_ignores_stale_process_temp_names() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("AGENTS.md");
    let stale = home
        .path()
        .join(format!(".AGENTS.md.note.{}.tmp", std::process::id()));
    fs::write(&stale, "left by an interrupted startup").unwrap();
    fs::write(&path, "personal instructions").unwrap();
    crate::working_note::write(home.path(), true).unwrap();
    assert_eq!(
        crate::working_note::strip(&fs::read_to_string(&path).unwrap()),
        "personal instructions"
    );
    assert_eq!(
        fs::read_to_string(&stale).unwrap(),
        "left by an interrupted startup"
    );
    assert_eq!(fs::read_dir(home.path()).unwrap().count(), 2);
}

#[cfg(unix)]
#[test]
fn startup_and_editor_replacement_preserve_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("AGENTS.md");
    fs::write(&path, "personal instructions").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    crate::working_note::write(home.path(), true).unwrap();
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o640
    );
    write_atomic(&path, "edited instructions").unwrap();
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o640
    );
    assert_eq!(fs::read_dir(home.path()).unwrap().count(), 1);
}

#[test]
fn startup_and_editor_replacement_preserve_private_access() {
    use std::io::Write;
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("AGENTS.md");
    let mut original = vw_platform::create_private_new(&path).unwrap();
    original.write_all(b"personal instructions").unwrap();
    drop(original);
    assert!(vw_platform::is_user_only(&path).unwrap());
    crate::working_note::write(home.path(), true).unwrap();
    assert!(vw_platform::is_user_only(&path).unwrap());
    write_atomic(&path, "edited instructions").unwrap();
    assert!(vw_platform::is_user_only(&path).unwrap());
    assert_eq!(fs::read_to_string(path).unwrap(), "edited instructions");
    assert_eq!(fs::read_dir(home.path()).unwrap().count(), 1);
}
