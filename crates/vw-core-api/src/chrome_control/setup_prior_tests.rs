//! What Connect finds before it writes: Windows key values and host-folder manifests over real scratch files.
//! The Windows shapes run on every OS because the judgement only reads files; the registry write that follows has its
//! own tests in vw-platform's extension registry tests.
//!
//! PREDICT (before the fix): there is no judgement at all. A key or manifest left by another setup whose file or
//! helper program no longer exists is refused like a live one ("belongs to another setup" / "set up by another Vibe
//! Work") with "Try Connect again", which cannot help; a live one is refused without naming where it is.
use super::*;

struct Tmp(PathBuf);
impl Tmp {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("vw-prior-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}
impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn manifest(helper: &Path) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({"name": HOST_NAME, "path": helper, "type": "stdio"}))
        .unwrap()
}

fn read(path: &Path) -> io::Result<Option<Vec<u8>>> {
    read_foreign(path)
}

const KEY: &str = r"HKCU\Software\Google\Chrome\NativeMessagingHosts\com.vibework.chrome_control";

#[test]
fn a_key_naming_a_missing_manifest_is_replaced_and_said_once() {
    let t = Tmp::new("missing-manifest");
    let state = t.0.join("state");
    let ours = state.join("bin/chrome-control");
    let own = state.join("host").join(format!("{HOST_NAME}.json"));
    let old =
        t.0.join("old-lane/helper-install/native-host-manifest.json");
    let value = old.to_string_lossy().into_owned();
    let replaced = registry("Google Chrome", KEY, Some(&value), &own, &ours, read)
        .unwrap()
        .expect("a missing manifest is stale");
    assert_eq!(replaced.registry_value.as_deref(), Some(value.as_str()));
    assert_eq!(
        replaced.notice,
        format!(
            "Connected. An old Chrome connection pointed to a file that no longer exists \
             ({value}); Vibe Work replaced it."
        )
    );

    // Edge's key naming this installation's own manifest path while that file is gone: rewritten by the ordinary
    // rule (no exact-value match needed), and still said once.
    let own_value = own.to_string_lossy().into_owned();
    let replaced = registry("Microsoft Edge", KEY, Some(&own_value), &own, &ours, read)
        .unwrap()
        .expect("our own missing manifest is said too");
    assert_eq!(replaced.registry_value, None);
    assert!(
        replaced
            .notice
            .starts_with("Connected. An old Edge connection pointed to")
    );
}

#[test]
fn a_key_whose_manifest_names_a_missing_helper_is_replaced() {
    let t = Tmp::new("missing-helper");
    let state = t.0.join("state");
    let ours = state.join("bin/chrome-control");
    let own = state.join("host").join(format!("{HOST_NAME}.json"));
    let other = t.0.join("other/host.json");
    std::fs::create_dir_all(other.parent().unwrap()).unwrap();
    let gone_helper = t.0.join("other/bin/chrome-control");
    std::fs::write(&other, manifest(&gone_helper)).unwrap();
    let value = other.to_string_lossy().into_owned();
    let replaced = registry("Google Chrome", KEY, Some(&value), &own, &ours, read)
        .unwrap()
        .expect("a manifest naming a missing helper is stale");
    assert_eq!(replaced.registry_value.as_deref(), Some(value.as_str()));
    assert!(
        replaced.notice.contains(&gone_helper.display().to_string()),
        "{}",
        replaced.notice
    );
    // A relative helper path is resolved against the manifest's folder.
    std::fs::write(
        &other,
        br#"{"name":"com.vibework.chrome_control","path":"bin/chrome-control"}"#,
    )
    .unwrap();
    assert!(
        registry("Google Chrome", KEY, Some(&value), &own, &ours, read)
            .unwrap()
            .is_some()
    );
}

#[test]
fn a_key_naming_another_live_setup_is_refused_with_its_path_and_the_way_out() {
    let t = Tmp::new("foreign-live");
    let state = t.0.join("state");
    let ours = state.join("bin/chrome-control");
    let own = state.join("host").join(format!("{HOST_NAME}.json"));
    let other = t.0.join("other/host.json");
    let helper = t.0.join("other/bin/chrome-control");
    std::fs::create_dir_all(helper.parent().unwrap()).unwrap();
    std::fs::write(&helper, b"helper").unwrap();
    std::fs::write(&other, manifest(&helper)).unwrap();
    let value = other.to_string_lossy().into_owned();
    let error = registry("Google Chrome", KEY, Some(&value), &own, &ours, read).unwrap_err();
    assert_eq!(
        error,
        format!(
            "Chrome is connected to another Vibe Work setup ({value}). Open that setup and use Remove in Settings > \
             Browser, or delete the registry key HKEY_CURRENT_USER\\Software\\Google\\Chrome\\NativeMessagingHosts\\\
             com.vibework.chrome_control, then retry Connect"
        )
    );
    assert!(!error.contains("Try Connect again"));
    // An unrecognized manifest of another setup is refused the same way, never replaced.
    std::fs::write(&other, b"not json").unwrap();
    assert!(registry("Google Chrome", KEY, Some(&value), &own, &ours, read).is_err());
}

#[test]
fn nothing_there_or_this_installations_own_live_key_goes_on_silently() {
    let t = Tmp::new("own-live");
    let state = t.0.join("state");
    let ours = state.join("bin/chrome-control");
    let own = state.join("host").join(format!("{HOST_NAME}.json"));
    assert_eq!(
        registry("Google Chrome", KEY, None, &own, &ours, read),
        Ok(None)
    );
    assert_eq!(
        registry("Google Chrome", KEY, Some("  "), &own, &ours, read),
        Ok(None)
    );
    std::fs::create_dir_all(ours.parent().unwrap()).unwrap();
    std::fs::create_dir_all(own.parent().unwrap()).unwrap();
    std::fs::write(&ours, b"helper").unwrap();
    std::fs::write(&own, manifest(&ours)).unwrap();
    // Forward slashes name the same file to Windows.
    let value = own.to_string_lossy().replace('\\', "/");
    assert_eq!(
        registry("Microsoft Edge", KEY, Some(&value), &own, &ours, read),
        Ok(None)
    );
}

#[test]
fn a_host_folder_manifest_naming_a_missing_helper_is_replaced_a_live_one_named() {
    let t = Tmp::new("folder");
    let ours = t.0.join("state/bin/chrome-control");
    let file =
        t.0.join("NativeMessagingHosts")
            .join(format!("{HOST_NAME}.json"));
    assert_eq!(folder("Google Chrome", &file, None, &ours), Ok(None));

    let gone_helper = t.0.join("dev/bin/chrome-control");
    let bytes = manifest(&gone_helper);
    let replaced = folder("Google Chrome", &file, Some(&bytes), &ours)
        .unwrap()
        .unwrap();
    assert_eq!(replaced.manifest_bytes.as_deref(), Some(bytes.as_slice()));
    assert_eq!(replaced.registry_value, None);
    assert!(
        replaced
            .notice
            .contains("helper program that no longer exists"),
        "{}",
        replaced.notice
    );

    std::fs::create_dir_all(gone_helper.parent().unwrap()).unwrap();
    std::fs::write(&gone_helper, b"helper").unwrap();
    let error = folder("Microsoft Edge", &file, Some(&bytes), &ours).unwrap_err();
    assert_eq!(
        error,
        format!(
            "Edge is connected to another Vibe Work setup ({}). Open that setup and use Remove in Settings > \
             Browser, or delete the file {}, then retry Connect",
            gone_helper.display(),
            file.display()
        )
    );
    assert!(folder("Google Chrome", &file, Some(b"garbled"), &ours).is_err());

    // This installation's own leftover: silent while its helper copy exists, said once when it was gone.
    let own = manifest(&ours);
    assert!(
        folder("Google Chrome", &file, Some(&own), &ours)
            .unwrap()
            .is_some()
    );
    std::fs::create_dir_all(ours.parent().unwrap()).unwrap();
    std::fs::write(&ours, b"helper").unwrap();
    assert_eq!(folder("Google Chrome", &file, Some(&own), &ours), Ok(None));
}
