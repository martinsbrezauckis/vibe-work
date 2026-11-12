//! Pinned real-Vibe selector with an explicit candidate option for launcher upgrade tests.
//! Uses the selected launcher's associated Python; called before any test session or model starts.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

pub const PIN: &str = include_str!("../../../../tools/public/codegen/vibe.pin");

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Runtime {
    pub server: PathBuf,
    pub python: PathBuf,
    pub vibe_version: String,
    pub python_version: String,
}

pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .unwrap()
}

pub fn default_server(root: &Path) -> PathBuf {
    root.join(format!("data/vibe-{}/venv", PIN.trim()))
        .join(if cfg!(windows) {
            "Scripts/vibe-app-server.exe"
        } else {
            "bin/vibe-app-server"
        })
}

pub fn selected() -> Result<Runtime, String> {
    let explicit = [
        "VW_TEST_VIBE_SERVER",
        "VW_LIVE_SERVER_BIN",
        "VW_DEV_SERVER_BIN",
    ]
    .into_iter()
    .find_map(|name| std::env::var_os(name).filter(|v| !v.is_empty()));
    let candidate = std::env::var("VW_TEST_VIBE_CANDIDATE").ok();
    let expected = selected_version(explicit.is_some(), candidate.as_deref())?;
    checked_version(
        &explicit.map_or_else(|| default_server(&repo_root()), PathBuf::from),
        expected,
    )
}

pub fn checked(server: &Path) -> Result<Runtime, String> {
    checked_version(server, PIN.trim())
}

fn selected_version(explicit: bool, candidate: Option<&str>) -> Result<&str, String> {
    match candidate {
        None => Ok(PIN.trim()),
        Some(version @ ("3.0.4" | "3.1.0" | "3.1.1" | "3.1.2")) if explicit => Ok(version),
        _ => Err("VW_TEST_VIBE_CANDIDATE requires an explicit server and an audited version (3.0.4, 3.1.0, 3.1.1 or 3.1.2)".into()),
    }
}

fn checked_version(server: &Path, expected: &str) -> Result<Runtime, String> {
    static CHECKED: OnceLock<Mutex<HashMap<PathBuf, Runtime>>> = OnceLock::new();
    if !server.is_absolute() || !server.is_file() {
        return Err(format!(
            "Pinned Vibe {} server missing: {}. Install data/vibe-{}/venv or set VW_LIVE_SERVER_BIN to the pinned server.",
            PIN.trim(),
            server.display(),
            PIN.trim()
        ));
    }
    let mut cache = CHECKED.get_or_init(Mutex::default).lock().unwrap();
    if let Some(runtime) = cache.get(server) {
        require_version(
            server,
            &runtime.vibe_version,
            &runtime.python_version,
            expected,
        )?;
        return Ok(runtime.clone());
    }
    let python = vw_supervisor::server_python(server).ok_or_else(|| {
        format!(
            "Cannot identify the Python belonging to selected Vibe server {}",
            server.display()
        )
    })?;
    let output = std::process::Command::new(&python).args(["-I", "-c",
        "import importlib.metadata,json,platform; print(json.dumps([importlib.metadata.version('mistral-vibe'),platform.python_version()]))"])
        .output().map_err(|e| format!("Cannot inspect {}: {e}", python.display()))?;
    if !output.status.success() {
        return Err(format!(
            "Cannot inspect selected Vibe {} (interpreter exit {})",
            server.display(),
            output.status
        ));
    }
    let (vibe_version, python_version): (String, String) = serde_json::from_slice(&output.stdout)
        .map_err(|_| {
        format!(
            "Cannot read Vibe/Python versions for {} (interpreter {}, exit {})",
            server.display(),
            python.display(),
            output.status
        )
    })?;
    require_version(server, &vibe_version, &python_version, expected)?;
    let runtime = Runtime {
        server: server.to_path_buf(),
        python,
        vibe_version,
        python_version,
    };
    cache.insert(server.to_path_buf(), runtime.clone());
    Ok(runtime)
}

fn require_version(server: &Path, vibe: &str, python: &str, expected: &str) -> Result<(), String> {
    if vibe != expected {
        return Err(format!(
            "Selected Vibe server {} has Vibe {vibe} / Python {python}; required Vibe {}. Refusing to run tests against another runtime.",
            server.display(),
            expected
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn old_and_future_versions_are_refused_with_the_found_version() {
        for found in ["2.26.1", "3.0.4", "3.1.0", "999.0.0"] {
            let error = require_version(Path::new("selected-server"), found, "3.12.13", PIN.trim())
                .unwrap_err();
            assert!(
                error.contains(found) && error.contains(PIN.trim()) && error.contains("3.12.13")
            );
        }
        assert!(
            require_version(
                Path::new("selected-server"),
                PIN.trim(),
                "3.14.8",
                PIN.trim()
            )
            .is_ok()
        );
    }

    #[test]
    fn candidate_requires_explicit_selection_and_still_checks_the_installed_version() {
        assert_eq!(selected_version(true, None).unwrap(), PIN.trim());
        assert_eq!(selected_version(true, Some("3.1.0")).unwrap(), "3.1.0");
        assert!(selected_version(false, Some("3.1.0")).is_err());
        assert_eq!(selected_version(true, Some("3.1.1")).unwrap(), "3.1.1");
        assert_eq!(selected_version(true, Some("3.1.2")).unwrap(), "3.1.2");
        assert!(selected_version(true, Some("999.0.0")).is_err());
        assert!(require_version(Path::new("candidate"), "3.1.0", "3.14.8", "3.1.0").is_ok());
        assert!(require_version(Path::new("candidate"), "3.0.4", "3.14.8", "3.1.0").is_err());
    }
}
