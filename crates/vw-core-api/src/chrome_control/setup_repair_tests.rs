//! Registration recovery after a browser profile reset; real setup in disposable homes only.
use super::*;

fn manifest(s: &Scratch) -> PathBuf {
    targets(&s.root.join("home"), &s.config())[0]
        .hosts
        .as_ref()
        .unwrap()
        .join(format!("{HOST_NAME}.json"))
}

#[test]
fn status_repairs_missing_and_changed_recorded_manifests() {
    let s = Scratch::new("repair-status");
    install(&s.config(), &s.state(), &s.vibe_home()).unwrap();
    let path = manifest(&s);
    let expected = std::fs::read(&path).unwrap();
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    super::super::view(&s.config(), Some(&s.state()), &s.vibe_home());
    assert_eq!(std::fs::read(&path).unwrap(), expected);
    let mut changed: serde_json::Value = serde_json::from_slice(&expected).unwrap();
    changed["allowed_origins"] = serde_json::json!([]);
    std::fs::write(&path, changed.to_string()).unwrap();
    super::super::view(&s.config(), Some(&s.state()), &s.vibe_home());
    assert_eq!(std::fs::read(&path).unwrap(), expected);
}

#[test]
fn refresh_refuses_a_linked_browser_ancestor() {
    let s = Scratch::new("repair-ancestor");
    install(&s.config(), &s.state(), &s.vibe_home()).unwrap();
    let path = manifest(&s);
    let data = path.parent().unwrap().parent().unwrap();
    let outside = s.root.join("foreign");
    std::fs::rename(data, &outside).unwrap();
    std::os::unix::fs::symlink(&outside, data).unwrap();
    let foreign = outside
        .join("NativeMessagingHosts")
        .join(format!("{HOST_NAME}.json"));
    std::fs::write(&foreign, b"foreign").unwrap();
    assert!(super::super::super::helper_refresh::refresh(&s.config(), &s.state()).is_err());
    assert_eq!(std::fs::read(&foreign).unwrap(), b"foreign");
}

#[test]
fn refresh_refuses_another_writers_manifest() {
    let s = Scratch::new("repair-foreign");
    install(&s.config(), &s.state(), &s.vibe_home()).unwrap();
    let path = manifest(&s);
    let foreign = host_manifest(&s.root.join("another/bin/helper"), EXTENSION_ID);
    std::fs::write(&path, &foreign).unwrap();
    assert!(super::super::super::helper_refresh::refresh(&s.config(), &s.state()).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), foreign);
}

#[cfg(feature = "your-edge")]
#[test]
fn repeated_edge_setup_repairs_its_missing_registration() {
    let s = Scratch::new("repair-edge");
    install(&s.config(), &s.state(), &s.vibe_home()).unwrap();
    let path = targets(&s.root.join("home"), &s.config())[1]
        .hosts
        .as_ref()
        .unwrap()
        .join(format!("{HOST_NAME}.json"));
    let expected = std::fs::read(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    register_edge(&s.config(), &s.state()).unwrap();
    assert_eq!(std::fs::read(path).unwrap(), expected);
}

#[test]
fn repair_refuses_a_manifest_link_and_a_forged_receipt_path() {
    let s = Scratch::new("repair-link");
    install(&s.config(), &s.state(), &s.vibe_home()).unwrap();
    let path = manifest(&s);
    let outside = s.root.join("foreign.json");
    std::fs::write(&outside, b"keep").unwrap();
    std::fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(&outside, &path).unwrap();
    let view = super::super::view(&s.config(), Some(&s.state()), &s.vibe_home());
    assert_eq!(view.state, SetupState::RepairRequired);
    assert_eq!(std::fs::read(&outside).unwrap(), b"keep");
    std::fs::remove_file(&path).unwrap();
    let mut receipt = read_receipt(&s.state()).unwrap();
    receipt.files.push(outside.clone());
    std::fs::write(
        s.state().join(RECEIPT),
        serde_json::to_vec(&receipt).unwrap(),
    )
    .unwrap();
    super::super::super::helper_refresh::refresh(&s.config(), &s.state()).unwrap();
    assert_eq!(std::fs::read(outside).unwrap(), b"keep");
}

#[test]
fn first_setup_preserves_an_existing_host_and_link() {
    for link in [false, true] {
        let s = Scratch::new(if link { "setup-link" } else { "setup-foreign" });
        let path = manifest(&s);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let foreign = s.root.join("foreign.json");
        std::fs::write(&foreign, b"keep").unwrap();
        if link {
            std::os::unix::fs::symlink(&foreign, &path).unwrap();
        } else {
            std::fs::write(&path, b"keep").unwrap();
        }
        assert!(install(&s.config(), &s.state(), &s.vibe_home()).is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"keep");
        assert_eq!(std::fs::read(foreign).unwrap(), b"keep");
    }
}

#[test]
fn remove_preserves_a_registration_replaced_by_another_installation() {
    let s = Scratch::new("remove-foreign-host");
    install(&s.config(), &s.state(), &s.vibe_home()).unwrap();
    let path = manifest(&s);
    let foreign = host_manifest(&s.root.join("another/bin/helper"), EXTENSION_ID);
    std::fs::write(&path, &foreign).unwrap();
    let _ = remove(&s.config(), &s.state());
    assert_eq!(std::fs::read(path).unwrap(), foreign);
}

#[test]
fn a_large_helper_does_not_limit_registration_checks_or_rollback() {
    let s = Scratch::new("repair-large-helper");
    let package = s.root.join("package/bin").join(helper_name());
    std::fs::write(package, vec![42; 2 * 1024 * 1024]).unwrap();
    install(&s.config(), &s.state(), &s.vibe_home()).unwrap();
    let path = manifest(&s);
    std::fs::remove_file(&path).unwrap();
    assert_eq!(
        super::super::view(&s.config(), Some(&s.state()), &s.vibe_home()).state,
        SetupState::WaitingForExtension
    );
    assert!(path.is_file());
    remove(&s.config(), &s.state()).unwrap();
    std::fs::write(path, b"foreign").unwrap();
    assert!(install(&s.config(), &s.state(), &s.vibe_home()).is_err());
    assert!(!s.state().join("bin").join(helper_name()).exists());
}

#[test]
fn a_linked_helper_is_not_published_in_a_repaired_registration() {
    let s = Scratch::new("repair-linked-helper");
    install(&s.config(), &s.state(), &s.vibe_home()).unwrap();
    let helper = s.state().join("bin").join(helper_name());
    std::fs::remove_file(&helper).unwrap();
    std::os::unix::fs::symlink(s.root.join("package/bin").join(helper_name()), helper).unwrap();
    let path = manifest(&s);
    std::fs::remove_file(&path).unwrap();
    assert_eq!(
        super::super::view(&s.config(), Some(&s.state()), &s.vibe_home()).state,
        SetupState::RepairRequired
    );
    assert!(!path.exists());
}

#[test]
fn changed_resource_or_installed_bytes_cannot_regain_native_registration() {
    use sha2::{Digest, Sha256};
    for resource in [false, true] {
        let s = Scratch::new(&format!("repair-changed-{resource}"));
        let mut config = s.config();
        config.helper_sha256 = Some(format!("{:x}", Sha256::digest(b"helper")));
        install(&config, &s.state(), &s.vibe_home()).unwrap();
        let path = manifest(&s);
        std::fs::remove_file(&path).unwrap();
        let helper = if resource {
            s.root.join("package/bin")
        } else {
            s.state().join("bin")
        };
        std::fs::write(helper.join(helper_name()), b"recorder").unwrap();
        let error = repair_registrations(&config, &s.state(), None).unwrap_err();
        assert!(error.contains("Reinstall Vibe Work."), "{error}");
        assert!(!path.exists());
    }
}
