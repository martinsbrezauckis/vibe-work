//! Validates the explicit disposable installed-app profile once per process.
//! Depends on private folders, webview policy and `serde_json`; called by app startup and preview storage.
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const ENV: &str = "VW_LIVE_SETUP_PROFILE";
static ROOT: OnceLock<Result<Option<PathBuf>, String>> = OnceLock::new();

/// An opt-in, precreated private run root with the setup script's nonsecret marker.
///
/// # Errors
/// The supplied folder is not a private disposable profile with the expected marker.
pub fn root() -> Result<Option<PathBuf>, String> {
    ROOT.get_or_init(|| {
        std::env::var_os(ENV)
            .map(PathBuf::from)
            .map(|path| validate(&path))
            .transpose()
    })
    .clone()
}

fn validate(path: &Path) -> Result<PathBuf, String> {
    #[cfg(windows)]
    if crate::webview::shared_user_data_folder() {
        return Err(format!(
            "{ENV} refuses a WebView2 user-data override or policy"
        ));
    }
    if !path.is_absolute() || path.parent().is_none() {
        return Err(format!("{ENV} must name an absolute disposable run folder"));
    }
    // Do not follow ancestor symlinks/junctions into a real user profile.
    for ancestor in path.ancestors() {
        let metadata = std::fs::symlink_metadata(ancestor)
            .map_err(|_| format!("{ENV} must name a precreated run folder"))?;
        let linked = metadata.file_type().is_symlink();
        #[cfg(windows)]
        let linked = {
            use std::os::windows::fs::MetadataExt;
            linked || metadata.file_attributes() & 0x400 != 0
        };
        if linked || !metadata.is_dir() {
            return Err(format!("{ENV} cannot contain a symlink or junction"));
        }
    }
    let marker = path.join("live-profile.json");
    let metadata = std::fs::symlink_metadata(&marker)
        .map_err(|_| format!("{ENV} needs its live-profile.json marker"))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 128 {
        return Err(format!("{ENV} has an invalid profile marker"));
    }
    let content =
        std::fs::read(&marker).map_err(|_| format!("{ENV} profile marker cannot be read"))?;
    let value: serde_json::Value = serde_json::from_slice(&content)
        .map_err(|_| format!("{ENV} profile marker is not valid JSON"))?;
    if value != serde_json::json!({ "schema": 1, "purpose": "vibe-work-live-setup" }) {
        return Err(format!("{ENV} profile marker is not recognized"));
    }
    crate::ensure_private_dir(path)
        .map_err(|_| format!("{ENV} folder is not private to this user"))?;
    for name in ["data", "local-data", "webview", "cli-vibe", "shared-vault"] {
        crate::ensure_private_dir(&path.join(name))
            .map_err(|_| format!("{ENV} cannot prepare its private {name} folder"))?;
    }
    Ok(path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_requires_private_owned_run_marker() {
        let run = tempfile::tempdir().expect("scratch run");
        let path = run.path().canonicalize().expect("absolute run");
        assert!(validate(&path).is_err());
        let marker = path.join("live-profile.json");
        std::fs::write(&marker, b"{}").expect("write wrong marker");
        assert!(validate(&path).is_err());
        std::fs::write(&marker, br#"{"schema":1,"purpose":"vibe-work-live-setup"}"#)
            .expect("write valid marker");
        assert_eq!(validate(&path).expect("owned run"), path);
        assert!(validate(Path::new("relative-run")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn profile_refuses_linked_roots_and_markers() {
        use std::os::unix::fs::symlink;
        let scratch = tempfile::tempdir().expect("scratch");
        let real = scratch.path().join("real");
        std::fs::create_dir(&real).expect("real");
        let marker = real.join("live-profile.json");
        std::fs::write(&marker, br#"{"schema":1,"purpose":"vibe-work-live-setup"}"#)
            .expect("marker");
        let link = scratch.path().join("link");
        symlink(&real, &link).expect("link");
        assert!(validate(&link).is_err());
        std::fs::remove_file(&marker).expect("remove marker");
        symlink(scratch.path().join("absent"), &marker).expect("marker link");
        assert!(validate(&real).is_err());
    }
}
