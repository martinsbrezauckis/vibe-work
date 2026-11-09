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

#[cfg(any(windows, test))]
enum RegistryValue {
    Dword(u32),
    String(String),
    Other,
}

#[cfg(any(windows, test))]
fn registry_policy(
    value: impl Fn(&str) -> Option<RegistryValue>,
    list: impl Fn(&str) -> Vec<RegistryValue>,
) -> Value {
    let mut values = serde_json::Map::new();
    if let Some(RegistryValue::Dword(value)) = value("NativeMessagingUserLevelHosts") {
        values.insert(
            "NativeMessagingUserLevelHosts".into(),
            Value::Bool(value != 0),
        );
    }
    if let Some(RegistryValue::String(text)) = value("ExtensionSettings")
        && let Ok(value) = serde_json::from_str(&text)
    {
        values.insert("ExtensionSettings".into(), value);
    }
    for name in [
        "NativeMessagingBlocklist",
        "NativeMessagingAllowlist",
        "ExtensionInstallBlocklist",
        "ExtensionInstallAllowlist",
    ] {
        let entries: Vec<_> = list(name)
            .into_iter()
            .filter_map(|value| match value {
                RegistryValue::String(text) => Some(Value::String(text.trim().to_owned())),
                _ => None,
            })
            .collect();
        if !entries.is_empty() {
            values.insert(name.into(), Value::Array(entries));
        }
    }
    Value::Object(values)
}

#[cfg(windows)]
fn registry_value(raw: &winreg::RegValue) -> RegistryValue {
    use winreg::{
        enums::{REG_DWORD, REG_SZ},
        types::FromRegValue,
    };
    match raw.vtype {
        REG_DWORD => u32::from_reg_value(raw).map_or(RegistryValue::Other, RegistryValue::Dword),
        REG_SZ => String::from_reg_value(raw).map_or(RegistryValue::Other, RegistryValue::String),
        _ => RegistryValue::Other,
    }
}

#[cfg(windows)]
fn read_key(root: &winreg::RegKey) -> Value {
    registry_policy(
        |name| {
            root.get_raw_value(name)
                .ok()
                .map(|raw| registry_value(&raw))
        },
        |name| {
            root.open_subkey(name).ok().map_or_else(Vec::new, |key| {
                key.enum_values()
                    .filter_map(Result::ok)
                    .map(|(_, raw)| registry_value(&raw))
                    .collect()
            })
        },
    )
}

#[cfg(windows)]
pub(super) fn read() -> Vec<Value> {
    [winreg::HKLM, winreg::HKCU]
        .into_iter()
        .filter_map(|hive| {
            hive.open_subkey(r"Software\Policies\Microsoft\Edge")
                .ok()
                .map(|key| read_key(&key))
        })
        .collect()
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub(super) fn read() -> Vec<Value> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn registry_values_keep_types_unicode_and_only_recognized_lists() {
        let values = |name: &str| match name {
            "NativeMessagingUserLevelHosts" => Some(RegistryValue::Dword(0)),
            "ExtensionSettings" => Some(RegistryValue::String(
                r#"{"*":{"installation_mode":"blocked"}}"#.into(),
            )),
            _ => None,
        };
        let lists = |name: &str| match name {
            "NativeMessagingAllowlist" => vec![
                RegistryValue::String("host.Mārtiņš ; $`&".into()),
                RegistryValue::Other,
            ],
            "ExtensionInstallBlocklist" => vec![RegistryValue::String("  *  ".into())],
            _ => Vec::new(),
        };
        assert_eq!(
            registry_policy(values, lists),
            json!({
                "NativeMessagingUserLevelHosts":false,
                "ExtensionSettings":{"*":{"installation_mode":"blocked"}},
                "NativeMessagingAllowlist":["host.Mārtiņš ; $`&"],
                "ExtensionInstallBlocklist":["*"]
            })
        );
        assert_eq!(
            registry_policy(|_| Some(RegistryValue::Other), |_| Vec::new()),
            json!({})
        );
        assert_eq!(
            registry_policy(
                |name| (name == "NativeMessagingUserLevelHosts").then_some(RegistryValue::Dword(2)),
                |_| Vec::new()
            ),
            json!({"NativeMessagingUserLevelHosts":true})
        );
        assert_eq!(
            registry_policy(
                |_| Some(RegistryValue::String("invalid JSON".into())),
                |_| Vec::new()
            ),
            json!({})
        );
    }

    #[cfg(windows)]
    #[test]
    fn native_registry_reader_uses_utf16_and_ignores_other_value_types() {
        struct Scratch(String);
        impl Drop for Scratch {
            fn drop(&mut self) {
                let _ = winreg::HKCU.delete_subkey_all(&self.0);
            }
        }
        let scratch = Scratch(format!(
            r"Software\VibeWorkTests\EdgePolicy-{}",
            std::process::id()
        ));
        let (key, _) = winreg::HKCU.create_subkey(&scratch.0).unwrap();
        key.set_value("NativeMessagingUserLevelHosts", &0u32)
            .unwrap();
        key.set_value(
            "ExtensionSettings",
            &r#"{"*":{"installation_mode":"blocked"}}"#,
        )
        .unwrap();
        let (list, _) = key.create_subkey("NativeMessagingAllowlist").unwrap();
        list.set_value("1", &"host.Mārtiņš ; $`&").unwrap();
        list.set_value("2", &1u32).unwrap();
        assert_eq!(
            read_key(&key),
            json!({
                "NativeMessagingUserLevelHosts":false,
                "ExtensionSettings":{"*":{"installation_mode":"blocked"}},
                "NativeMessagingAllowlist":["host.Mārtiņš ; $`&"]
            })
        );
        key.set_value("ExtensionSettings", &vec!["wrong", "type"])
            .unwrap();
        key.set_value("NativeMessagingUserLevelHosts", &"0")
            .unwrap();
        assert_eq!(
            read_key(&key),
            json!({"NativeMessagingAllowlist":["host.Mārtiņš ; $`&"]})
        );
    }
}
