//! Native filesystem regressions for the unsupported-disposition fallback.
//! Injects only the extended-disposition error; opens, link counts, permissions and deletion stay real.

#![allow(clippy::unwrap_used)]

use std::fs;
use std::io;
use std::path::Path;

use cap_std::fs::Dir;

fn readonly(path: &Path, value: bool) {
    let mut perms = fs::metadata(path).unwrap().permissions();
    perms.set_readonly(value);
    fs::set_permissions(path, perms).unwrap();
}

#[test]
fn unsupported_disposition_removes_single_link_readonly_file() {
    for code in [1, 50, 87] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("file");
        fs::write(&path, b"owned").unwrap();
        readonly(&path, true);
        let parent = Dir::open_ambient_dir(temp.path(), cap_std::ambient_authority()).unwrap();
        super::remove_with(&parent, Path::new("file"), |_| {
            Err(io::Error::from_raw_os_error(code))
        })
        .unwrap();
        assert!(!path.exists());
    }
}

#[test]
fn unsupported_disposition_refuses_shared_readonly_file() {
    let temp = tempfile::tempdir().unwrap();
    let outside = temp.path().join("kept");
    let home = temp.path().join("home");
    fs::write(&outside, b"outside nonce").unwrap();
    fs::create_dir(&home).unwrap();
    fs::hard_link(&outside, home.join("link")).unwrap();
    readonly(&outside, true);
    let parent = Dir::open_ambient_dir(&home, cap_std::ambient_authority()).unwrap();
    let error = super::remove_with(&parent, Path::new("link"), |_| {
        Err(io::Error::from_raw_os_error(87))
    })
    .unwrap_err();
    let kept_readonly = fs::File::open(&outside)
        .unwrap()
        .metadata()
        .unwrap()
        .permissions()
        .readonly();
    readonly(&outside, false);
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    assert!(error.to_string().contains("multiple hard links"));
    assert!(
        kept_readonly,
        "fallback made the outside hard link writable"
    );
    assert!(home.join("link").exists());
    assert_eq!(fs::read(&outside).unwrap(), b"outside nonce");
}

#[test]
fn other_disposition_errors_do_not_clear_readonly() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("file");
    fs::write(&path, b"owned").unwrap();
    readonly(&path, true);
    let parent = Dir::open_ambient_dir(temp.path(), cap_std::ambient_authority()).unwrap();
    let error = super::remove_with(&parent, Path::new("file"), |_| {
        Err(io::Error::from_raw_os_error(5))
    })
    .unwrap_err();
    let kept_readonly = fs::metadata(&path).unwrap().permissions().readonly();
    readonly(&path, false);
    assert_eq!(error.raw_os_error(), Some(5));
    assert!(kept_readonly);
}
