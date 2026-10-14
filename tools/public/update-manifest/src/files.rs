//! The three commands on files: write a manifest, check a written one, verify one signature. The command line
//! (`main.rs`) only parses arguments into these requests; the tests call them directly.
//!
//! Extra rules here (on top of the crate root's): the release version equals the app config's `version` (what the
//! updater compares the announcement with), and for `write` the channel file's `pubkey` (what the builds embed)
//! equals the public key the signatures are checked with. `write` checks before it writes anything and replaces
//! the output atomically (temporary file, then rename).
//!
//! Depends on: the crate root. Called by: `main.rs`, the release tests.

use std::path::{Path, PathBuf};

use crate::{Refused, Release};

fn read(path: &Path, what: &str) -> Result<String, Refused> {
    std::fs::read_to_string(path)
        .map_err(|e| Refused(format!("the {what} {}: {e}", path.display())))
}

/// `version` is plain `X.Y.Z` and equals the app config's version.
///
/// # Errors
/// [`Refused`] on a different or unreadable version.
pub fn expect_app_version(version: &str, app_config: &Path) -> Result<(), Refused> {
    crate::plain_version(version)?;
    let app = crate::app_version(&read(app_config, "app config")?)?;
    if app == version {
        Ok(())
    } else {
        Err(Refused(format!(
            "the release version {version} differs from the app's version {app} in {} (the updater compares with that)",
            app_config.display()
        )))
    }
}

/// The public key text of a key file (the Tauri `.pub` text: base64 of the minisign public key).
///
/// # Errors
/// [`Refused`] when the file is missing or empty.
pub fn read_pubkey(path: &Path) -> Result<String, Refused> {
    let key = read(path, "public key file")?;
    if key.trim().is_empty() {
        return Err(Refused(format!("{} is empty", path.display())));
    }
    Ok(key.trim().to_owned())
}

/// Everything `write` needs.
#[derive(Debug, Clone)]
pub struct WriteRequest<'a> {
    /// The release version `X.Y.Z`.
    pub version: &'a str,
    /// The app's Tauri config (`app/tauri.conf.json`).
    pub app_config: &'a Path,
    /// The update channel file the builds embedded.
    pub channel: &'a Path,
    /// The release public key file.
    pub pubkey: &'a Path,
    /// The release notes (text, trimmed).
    pub notes: &'a Path,
    /// `YYYY-MM-DDTHH:MM:SSZ`; `None` = now.
    pub pub_date: Option<&'a str>,
    /// The folder with the three artifacts and their `.sig` files.
    pub artifacts: &'a Path,
    /// The manifest file to write.
    pub out: &'a Path,
}

/// Builds, checks and writes the manifest. Nothing is written on a refusal.
///
/// # Errors
/// [`Refused`] on the first broken rule.
pub fn write_manifest(request: &WriteRequest<'_>) -> Result<PathBuf, Refused> {
    expect_app_version(request.version, request.app_config)?;
    let key = read_pubkey(request.pubkey)?;
    let embedded = crate::channel_pubkey(&read(request.channel, "channel file")?)?;
    if embedded != key {
        return Err(Refused(format!(
            "the channel's pubkey (what the builds embed) is not the key in {}",
            request.pubkey.display()
        )));
    }
    let notes = read(request.notes, "notes file")?;
    let now = crate::now_rfc3339();
    let release = Release {
        version: request.version,
        notes: notes.trim(),
        pub_date: request.pub_date.unwrap_or(&now),
        pubkey: &key,
    };
    let text = crate::build(&release, request.artifacts)?;
    let out = request.out.to_path_buf();
    let temp = out.with_extension("json.tmp");
    std::fs::write(&temp, &text)
        .and_then(|()| std::fs::rename(&temp, &out))
        .map_err(|e| {
            let _ = std::fs::remove_file(&temp);
            Refused(format!("{}: {e}", out.display()))
        })?;
    Ok(out)
}

/// Checks a written manifest file against the artifacts (all of the crate root's rules plus the app version).
///
/// # Errors
/// [`Refused`] on the first broken rule.
pub fn check_manifest(
    version: &str,
    app_config: &Path,
    pubkey: &Path,
    artifacts: &Path,
    manifest: &Path,
) -> Result<(), Refused> {
    expect_app_version(version, app_config)?;
    let key = read_pubkey(pubkey)?;
    crate::check(&read(manifest, "manifest")?, artifacts, version, &key)
}

/// Verifies `<file>.sig` over `file` for `version`, bound to the file's name.
///
/// # Errors
/// [`Refused`] when anything is missing or the signature does not hold.
pub fn verify_file(version: &str, pubkey: &Path, file: &Path) -> Result<(), Refused> {
    let key = read_pubkey(pubkey)?;
    let data = std::fs::read(file).map_err(|e| Refused(format!("{}: {e}", file.display())))?;
    let mut sig = file.as_os_str().to_owned();
    sig.push(".sig");
    let signature = read(Path::new(&sig), "signature")?;
    let name = file.file_name().map(|n| n.to_string_lossy().into_owned());
    crate::verify_signature(&data, &signature, &key, version, name.as_deref())
}
