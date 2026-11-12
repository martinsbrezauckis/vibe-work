//! Native conditional registry-removal contracts using the existing disposable test root.
//! Exercises vw-platform registration transactions; run by Windows core tests.
use vw_platform::extension_registry::{Key, Native, Registry, create, undo};
const UPDATE_URL: &str = "https://clients2.google.com/service/update2/crx";

#[cfg(windows)]
#[test]
fn native_removal_preserves_a_preexisting_marker_value() {
    use winreg::HKCU;
    let root = super::TestRoot::new("removal-marker");
    let name = format!(r"{}\extension", root.key());
    assert!(create(&name, &Native).unwrap());
    let path = name.trim_start_matches("HKCU\\");
    let key = HKCU
        .open_subkey_with_flags(path, winreg::enums::KEY_ALL_ACCESS)
        .unwrap();
    key.set_value("vibe-work-removal-lock", &"foreign marker")
        .unwrap();
    assert!(!undo(Some(&name), &Native).unwrap(), "not ours");
    let key = HKCU.open_subkey(path).unwrap();
    assert_eq!(
        key.get_value::<String, _>("vibe-work-removal-lock")
            .unwrap(),
        "foreign marker"
    );
    assert_eq!(
        key.get_value::<String, _>("update_url").unwrap(),
        UPDATE_URL
    );
}

#[cfg(windows)]
#[test]
fn native_registry_creation_and_conditional_removal() {
    use winreg::{
        HKCU,
        enums::{KEY_READ, KEY_WRITE},
    };
    // Deletes its key and the empty shared test parent when dropped, also after a failed assertion.
    let test_root = super::TestRoot::new("external-cleanup");
    let root = test_root.key().trim_start_matches("HKCU\\").to_owned();
    let name = format!(r"HKCU\{root}\extension");
    let path = format!(r"{root}\extension");
    assert!(create(&name, &Native).unwrap());
    assert!(!create(&name, &Native).unwrap());
    assert!(undo(Some(&name), &Native).unwrap());
    for foreign in ["value", "child", "empty"] {
        let (key, _) = HKCU.create_subkey(&path).unwrap();
        if foreign == "value" {
            key.set_value("update_url", &"foreign").unwrap();
        }
        if foreign == "child" {
            key.create_subkey("child").unwrap();
        }
        assert!(
            !create(&name, &Native).unwrap(),
            "never adopt even an empty existing key"
        );
        drop(key);
        HKCU.delete_subkey_all(&path).unwrap(); // Test fixture cleanup only.
    }
    // A writer after the final ownership read must either be refused by Windows' transaction isolation,
    // or survive because our transaction refuses the conflicting delete/commit.
    for child in [false, true] {
        assert!(create(&name, &Native).unwrap());
        let checked = Native.open(&name).unwrap().unwrap();
        assert!(!checked.marker_exists().unwrap());
        assert!(checked.lock().unwrap());
        assert!(checked.owned().unwrap());
        assert!(checked.owned().unwrap());
        let writer = HKCU
            .open_subkey_with_flags(&path, KEY_READ | KEY_WRITE)
            .unwrap();
        let wrote = if child {
            writer.create_subkey("child").map(|_| ())
        } else {
            writer.set_value("update_url", &"foreign")
        };
        if checked.delete().is_ok() {
            let _ = checked.commit();
        } else {
            drop(checked);
        }
        if wrote.is_ok() {
            if child {
                assert!(writer.open_subkey("child").is_ok());
            } else {
                assert_eq!(
                    writer.get_value::<String, _>("update_url").unwrap(),
                    "foreign"
                );
            }
        }
        drop(writer);
        let _ = HKCU.delete_subkey_all(&path);
    }
    drop(test_root);
}
