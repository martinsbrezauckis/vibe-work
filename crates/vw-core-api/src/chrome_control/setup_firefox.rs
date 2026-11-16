//! Firefox sharing add-on registration. The owned automation window uses its independent `BiDi` connection.
//! Firefox's `allowed_extensions` identity is independent of Chromium origins; no profile preference is changed.
use super::{FIREFOX_EXTENSION_ID, HOST_NAME, Path};

pub(super) fn manifest(helper: &Path) -> Vec<u8> {
    serde_json::to_vec_pretty(&serde_json::json!({
        "name": HOST_NAME, "description": "Vibe Work extension helper", "path": helper,
        "type": "stdio", "allowed_extensions": [FIREFOX_EXTENSION_ID],
    }))
    .unwrap_or_default()
}

/// A foreign native registration is never replaced or added to our removal receipt. A registration that already
/// names this installation (the manifest file naming `helper`; on Windows the key's value naming `manifest_file`) is
/// one an earlier setup of this installation left behind when the app's data was removed: setup takes it back.
/// An unreadable one is refused like a foreign one.
pub(super) fn require_unregistered(
    config: &super::ChromeControlConfig,
    target: &super::places::Target<'_>,
    helper: &Path,
    manifest_file: &Path,
) -> Result<(), String> {
    const FOREIGN: &str = "Firefox is already set up by another Vibe Work";
    if let Some(hosts) = &target.hosts {
        let file = hosts.join(format!("{HOST_NAME}.json"));
        let before = super::files::read(config, &file)
            .map_err(|e| super::problem::io_message(config, &e))?;
        if let Some(bytes) = before
            && super::registration::owned_manifest(&bytes, &manifest(helper)).is_err()
        {
            return Err(FOREIGN.into());
        }
    } else if let Some(root) = target.registry {
        let output = vw_platform::process_command::hidden("reg")
            .args(["query", &format!(r"{root}\{HOST_NAME}"), "/ve"])
            .stderr(std::process::Stdio::null())
            .output()
            .map_err(|e| e.to_string())?;
        if output.status.success() {
            let native = super::windows_native_path(manifest_file);
            if !default_value_names(&String::from_utf8_lossy(&output.stdout), &native) {
                return Err(FOREIGN.into());
            }
        } else if output.status.code() != Some(1) {
            return Err("Firefox registration could not be checked".into());
        }
    }
    Ok(())
}

/// True when `reg query <key> /ve` output gives the key's default value as `manifest` (slashes and case
/// normalized). The value's display name is localized, so the line is found by its `REG_SZ` type.
fn default_value_names(output: &str, manifest: &str) -> bool {
    output.lines().any(|line| {
        line.split_once("REG_SZ").is_some_and(|(_, value)| {
            value
                .trim()
                .replace('/', "\\")
                .eq_ignore_ascii_case(&manifest.replace('/', "\\"))
        })
    })
}

#[cfg(test)]
mod tests {
    use super::default_value_names;

    /// The Windows registry shape of a leftover Firefox registration: the key's default value names this
    /// installation's manifest (taken back), another file, or nothing (refused).
    #[test]
    fn a_registry_value_naming_this_manifest_is_this_installations_own() {
        let manifest = r"C:\Users\p\AppData\Roaming\VibeWork\ChromeControl\firefox-host\h.json";
        let ours = format!(
            "\r\nHKEY_CURRENT_USER\\Software\\Mozilla\\NativeMessagingHosts\\h\r\n    (Default)    REG_SZ    {}\r\n",
            manifest.to_lowercase().replace('\\', "/")
        );
        assert!(default_value_names(&ours, manifest), "{ours}");
        let localized = format!("    (Standard)    REG_SZ    {manifest}\r\n");
        assert!(default_value_names(&localized, manifest));
        let other = r"    (Default)    REG_SZ    C:\Other\h.json";
        assert!(!default_value_names(other, manifest));
        let empty = "    (Default)    REG_SZ    \r\n";
        assert!(!default_value_names(empty, manifest));
        let unset = "    (Default)    REG_SZ    (value not set)\r\n";
        assert!(!default_value_names(unset, manifest));
        assert!(!default_value_names("", manifest));
    }
}
