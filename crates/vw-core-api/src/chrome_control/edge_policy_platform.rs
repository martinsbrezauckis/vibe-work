//! Local managed Edge policy sources. Read only; used by the optional browser setup diagnostics.
use serde_json::Value;

#[cfg(target_os = "linux")]
pub(super) fn read() -> Vec<Value> {
    read_dir(std::path::Path::new("/etc/opt/edge/policies/managed"))
}

#[cfg(target_os = "linux")]
fn read_dir(path: &std::path::Path) -> Vec<Value> {
    let Ok(entries) = std::fs::read_dir(path) else {
        return Vec::new();
    };
    let mut paths: Vec<_> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    paths.sort();
    let mut merged = serde_json::Map::new();
    for path in paths {
        if std::fs::metadata(&path).is_ok_and(|meta| meta.len() <= 1024 * 1024)
            && let Ok(bytes) = std::fs::read(path)
            && let Ok(Value::Object(values)) = serde_json::from_slice::<Value>(&bytes)
        {
            merged.extend(values);
        }
    }
    vec![Value::Object(merged)]
}

#[cfg(target_os = "macos")]
pub(super) fn read() -> Vec<Value> {
    let mut paths = vec![std::path::PathBuf::from(
        "/Library/Managed Preferences/com.microsoft.Edge.plist",
    )];
    if let Some(home) = std::env::var_os("HOME") {
        paths.push(
            std::path::PathBuf::from(home)
                .join("Library/Managed Preferences/com.microsoft.Edge.plist"),
        );
    }
    paths
        .into_iter()
        .filter_map(|path| {
            let out = std::process::Command::new("/usr/bin/plutil")
                .args(["-convert", "json", "-o", "-"])
                .arg(path)
                .output()
                .ok()?;
            out.status
                .success()
                .then(|| serde_json::from_slice(&out.stdout).ok())
                .flatten()
        })
        .collect()
}

#[cfg(windows)]
pub(super) fn read() -> Vec<Value> {
    use std::os::windows::process::CommandExt;
    [
        r"HKLM\Software\Policies\Microsoft\Edge",
        r"HKCU\Software\Policies\Microsoft\Edge",
    ]
    .into_iter()
    .filter_map(|root| {
        let out = std::process::Command::new("reg.exe")
            .args(["query", root, "/s"])
            .creation_flags(0x0800_0000)
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&out.stdout);
        let mut values = serde_json::Map::new();
        let mut section = "";
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with("HKEY_") {
                section = line.rsplit('\\').next().unwrap_or_default();
                continue;
            }
            let Some((name, raw)) = line.split_once("REG_") else {
                continue;
            };
            let name = name.trim();
            if let Some(value) = raw.strip_prefix("DWORD") {
                if name == "NativeMessagingUserLevelHosts" {
                    values.insert(name.into(), Value::Bool(value.trim() != "0x0"));
                }
            } else if let Some(value) = raw.strip_prefix("SZ") {
                if name == "ExtensionSettings" {
                    if let Ok(value) = serde_json::from_str(value.trim()) {
                        values.insert(name.into(), value);
                    }
                } else if [
                    "NativeMessagingBlocklist",
                    "NativeMessagingAllowlist",
                    "ExtensionInstallBlocklist",
                    "ExtensionInstallAllowlist",
                ]
                .contains(&section)
                {
                    values
                        .entry(section)
                        .or_insert_with(|| Value::Array(Vec::new()))
                        .as_array_mut()?
                        .push(Value::String(value.trim().into()));
                }
            }
        }
        Some(Value::Object(values))
    })
    .collect()
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub(super) fn read() -> Vec<Value> {
    Vec::new()
}
