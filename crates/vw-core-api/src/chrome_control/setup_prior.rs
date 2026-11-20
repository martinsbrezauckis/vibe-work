//! The Chrome or Edge browser-helper registration Connect finds before it writes its own. Called by setup before any
//! write, so the helper copy setup is about to place cannot make an old registration look alive.
//!
//! - Nothing there, or this installation's own live registration: Connect goes on as before.
//! - A stale registration (its manifest file, or the helper program its manifest names, no longer exists): Connect
//!   replaces it and the result line says so once ([`SetupNotice`]).
//! - A live registration of another setup: Connect refuses and names that setup's file and the step that clears it.
use super::{ChromeControlConfig, HOST_NAME, Path, PathBuf};
use std::{
    io,
    sync::{Arc, Mutex},
};
use vw_platform::sync::LockRecover;

/// A stale registration Connect may replace, and the value it must still find when it does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Stale {
    /// The Windows key's default value as read; the registry write replaces only this exact value.
    pub(super) registry_value: Option<String>,
    /// The host-folder manifest's bytes as read; the file write replaces only these exact bytes.
    pub(super) manifest_bytes: Option<Vec<u8>>,
    /// The result line's words for what was replaced.
    pub(super) notice: String,
}

/// The one-time result line of the last Connect that replaced a stale registration; shared by cloned configurations.
#[derive(Clone, Default)]
pub struct SetupNotice(Arc<Mutex<Option<String>>>);

impl SetupNotice {
    pub(super) fn set(&self, text: String) {
        *self.0.lock_or_recover() = Some(text);
    }
    /// Takes the line so it is shown once.
    pub(in crate::chrome_control) fn take(&self) -> Option<String> {
        self.0.lock_or_recover().take()
    }
}

fn short(browser: &str) -> &'static str {
    if browser == "Microsoft Edge" {
        "Edge"
    } else {
        "Chrome"
    }
}

fn missing_file(browser: &str, path: &str) -> String {
    format!(
        "Connected. An old {} connection pointed to a file that no longer exists ({path}); \
         Vibe Work replaced it.",
        short(browser)
    )
}

fn missing_helper(browser: &str, helper: &str) -> String {
    format!(
        "Connected. An old {} connection named a helper program that no longer exists \
         ({helper}); Vibe Work replaced it.",
        short(browser)
    )
}

/// The refusal for another setup's live registration. It ends with its own "retry Connect" step, so the
/// Settings line does not add "Try Connect again".
fn foreign(browser: &str, shown: &str, remove: &str) -> String {
    format!(
        "{} is connected to another Vibe Work setup ({shown}). Open that setup and use Remove in Settings > \
         Browser, or delete {remove}, then retry Connect",
        short(browser)
    )
}

/// True only when the OS says the path does not exist; a refused or failed check counts as present.
fn gone(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_err_and(|e| e.kind() == io::ErrorKind::NotFound)
}

/// The helper program a manifest names, resolved against the manifest's folder when relative (Windows allows that).
fn helper_of(bytes: &[u8], manifest: &Path) -> Option<PathBuf> {
    let value: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    if value["name"] != HOST_NAME {
        return None;
    }
    let path = PathBuf::from(value["path"].as_str().filter(|p| !p.is_empty())?);
    Some(if path.is_absolute() {
        path
    } else {
        manifest.parent()?.join(path)
    })
}

fn same_path(a: &str, b: &str) -> bool {
    a.replace('/', "\\")
        .eq_ignore_ascii_case(&b.replace('/', "\\"))
}

/// A host-folder registration (macOS, Linux): the manifest file itself. `ours` is this installation's helper path.
pub(super) fn folder(
    browser: &str,
    file: &Path,
    before: Option<&[u8]>,
    ours: &Path,
) -> Result<Option<Stale>, String> {
    let Some(bytes) = before else {
        return Ok(None);
    };
    let shown = file.display().to_string();
    let Some(helper) = helper_of(bytes, file) else {
        return Err(foreign(browser, &shown, "that file"));
    };
    let stale = || Stale {
        registry_value: None,
        manifest_bytes: Some(bytes.to_vec()),
        notice: missing_helper(browser, &helper.display().to_string()),
    };
    if helper == ours {
        // This installation's own leftover: taken back as before, and said once when its helper copy was gone.
        return Ok(gone(ours).then(stale));
    }
    if gone(&helper) {
        return Ok(Some(stale()));
    }
    Err(foreign(
        browser,
        &helper.display().to_string(),
        &format!("the file {shown}"),
    ))
}

/// A Windows registration: the key's default `value` names a manifest file. `manifest` is this installation's own
/// manifest file, `ours` its helper path, `key` the registry key for the refusal's words. `read` reads a manifest
/// (at most what a manifest needs).
pub(super) fn registry(
    browser: &str,
    key: &str,
    value: Option<&str>,
    manifest: &Path,
    ours: &Path,
    read: impl Fn(&Path) -> io::Result<Option<Vec<u8>>>,
) -> Result<Option<Stale>, String> {
    let Some(value) = value.filter(|v| !v.trim().is_empty()) else {
        return Ok(None);
    };
    let named = PathBuf::from(value);
    let own = same_path(value, &super::windows_native_path(manifest));
    let replace = |notice| {
        Ok(Some(Stale {
            // This installation's own key is rewritten by the ordinary rule; only another value needs the exact match.
            registry_value: (!own).then(|| value.to_owned()),
            manifest_bytes: None,
            notice,
        }))
    };
    if gone(&named) {
        return replace(missing_file(browser, value));
    }
    let refused = || {
        Err(foreign(
            browser,
            value,
            &format!(
                r"the registry key HKEY_CURRENT_USER\{}",
                key.trim_start_matches(r"HKCU\")
            ),
        ))
    };
    let bytes = match read(&named) {
        Ok(Some(bytes)) => bytes,
        Ok(None) if !own => return replace(missing_file(browser, value)),
        Ok(None) => return Ok(None),
        Err(_) if own => return Ok(None),
        Err(_) => return refused(),
    };
    let Some(helper) = helper_of(&bytes, &named) else {
        // This installation's own manifest is rewritten by the ordinary manifest rule.
        return if own { Ok(None) } else { refused() };
    };
    if same_path(&helper.to_string_lossy(), &ours.to_string_lossy()) {
        return if gone(ours) {
            replace(missing_helper(browser, &helper.display().to_string()))
        } else {
            Ok(None)
        };
    }
    if gone(&helper) {
        return replace(missing_helper(browser, &helper.display().to_string()));
    }
    if own {
        // Our own manifest file naming another helper is refused by the manifest rule with its own words.
        return Ok(None);
    }
    refused()
}

/// What Connect found for `browser` before writing anything. Windows reads the key; host folders read the file.
pub(super) fn check(
    config: &ChromeControlConfig,
    target: &super::places::Target<'_>,
    state: &Path,
) -> Result<Option<Stale>, String> {
    if target.name == "Mozilla Firefox" || std::env::var_os("NMH15_RED").is_some() {
        return Ok(None);
    }
    let helper = state.join("bin").join(super::helper_name());
    if let Some(hosts) = &target.hosts {
        let file = hosts.join(format!("{HOST_NAME}.json"));
        let before = super::files::read(config, &file)
            .map_err(|e| super::problem::io_message(config, &e))?;
        return folder(target.name, &file, before.as_deref(), &helper);
    }
    let Some(root) = target.registry else {
        return Ok(None);
    };
    let key = format!(r"{root}\{HOST_NAME}");
    let value = super::super::extension_registry::host_value(
        &key,
        &super::super::extension_registry::Native,
    )?;
    registry(
        target.name,
        &key,
        value.as_deref(),
        &super::manifest_path(state, target.name),
        &helper,
        read_foreign,
    )
}

/// Reads another setup's manifest without following it anywhere: a plain file of manifest size only.
fn read_foreign(path: &Path) -> io::Result<Option<Vec<u8>>> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    if !meta.is_file() || meta.len() > 1024 * 1024 {
        return Err(io::Error::other("not a plain manifest file"));
    }
    std::fs::read(path).map(Some)
}

#[cfg(test)]
#[path = "setup_prior_tests.rs"]
mod tests;
