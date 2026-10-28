//! Writes and checks `latest.json`, the static update manifest the app's updater reads from the GitHub release
//! channel. One release = one manifest with exactly three platforms:
//!
//! | key              | update artifact                         | starts with   |
//! |------------------|-----------------------------------------|---------------|
//! | `windows-x86_64` | the signed NSIS installer `*-setup.exe` | `MZ`          |
//! | `darwin-aarch64` | `*.app.tar.gz` of the stapled app       | gzip magic    |
//! | `linux-x86_64`   | the `*.AppImage`                        | ELF magic     |
//!
//! The updater plugin looks for `{os}-{arch}-{installer}` and then `{os}-{arch}`, so these plain keys serve the
//! NSIS install, the `.app` and the `AppImage` (a deb install finds the `AppImage` entry and only shows a link).
//!
//! **Everything fails closed and nothing is written on a refusal.** [`build`] assembles the manifest from one folder
//! that holds the three final artifacts and their `.sig` files (the Tauri CLI's `signer sign` output: base64 of the
//! minisign signature text), then runs [`check`] on its own output; [`check`] is also what re-validates a manifest
//! file before it is uploaded. The rules:
//! - `version` is a plain `X.Y.Z` and equals the expected version (the caller checks it against the app config);
//! - exactly the three platform keys; one artifact per platform, of the right kind (name suffix and magic bytes);
//! - asset names use only `A-Z a-z 0-9 . _ -` (GitHub rewrites other characters, e.g. a space becomes a dot, so the
//!   URL would miss the uploaded file) and contain `_<version>_` (a stale artifact of another build is refused);
//! - every URL is exactly `https://github.com/martinsbrezauckis/vibe-work/releases/download/v<version>/<asset>`;
//! - every signature equals the asset's `.sig` file, verifies over the asset's bytes with the release public key
//!   (prehashed minisign signatures only), and its trusted comment (covered by the signature) names
//!   `version:<version>` and `file:<asset name>`. The version rule is the app's own (`requireSignedVersion`: an old
//!   release can never be replayed as a new one); the file rule binds each signature to the asset it was made for;
//! - `notes` is a non-empty text and `pub_date` an RFC 3339 UTC time `YYYY-MM-DDTHH:MM:SSZ`; no other fields;
//! - the oldest system each package runs on ([`Minimums`]), each exactly when the release step passes it and then
//!   equal to it: `darwin-aarch64` `minimumSystemVersion` (the bundle's `LSMinimumSystemVersion`, `11.0`),
//!   `windows-x86_64` `minimumSystemVersion` (a Windows build, `10.0.17763`), `linux-x86_64` `minimumGlibc` (the
//!   highest glibc symbol version the `AppImage`'s programs need, `2.39`); dotted numbers. The app's updater does not
//!   download a package its system is older than (older apps ignore the fields). No other field is allowed.
//!
//! Modules: `files` (the commands on files, plus the app-version and channel-key rules), `time` (`pub_date`).
//! Depends on: minisign-verify, base64, semver, `serde_json`. Called by: `src/main.rs`; the app's updater format
//! test ([`render`]); the release tests.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use minisign_verify::{PublicKey, Signature};
use serde_json::{Map, Value};

mod files;
mod time;

pub use files::{
    WriteRequest, check_manifest, expect_app_version, read_pubkey, verify_file, write_manifest,
};
pub use time::{now_rfc3339, valid_pub_date};

/// The GitHub repository whose releases carry the update artifacts (`owner/name`).
pub const REPO: &str = "martinsbrezauckis/vibe-work";

/// The kind of update artifact a platform takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The NSIS installer (`*-setup.exe`), run by the updater in its update mode.
    NsisInstaller,
    /// The `.app` bundle as `*.app.tar.gz` (top folder `<name>.app`).
    AppArchive,
    /// The `AppImage`, replaced in place.
    AppImage,
}

impl Kind {
    fn suffix(self) -> &'static str {
        match self {
            Self::NsisInstaller => "-setup.exe",
            Self::AppArchive => ".app.tar.gz",
            Self::AppImage => ".AppImage",
        }
    }

    fn magic(self) -> &'static [u8] {
        match self {
            Self::NsisInstaller => b"MZ",
            Self::AppArchive => &[0x1f, 0x8b],
            Self::AppImage => b"\x7fELF",
        }
    }
}

/// The platforms of every release, in manifest order.
pub const PLATFORMS: [(&str, Kind); 3] = [
    ("windows-x86_64", Kind::NsisInstaller),
    ("darwin-aarch64", Kind::AppArchive),
    ("linux-x86_64", Kind::AppImage),
];

/// Why a manifest was not written or not accepted. The text names the rule and the file, never key material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused(pub String);

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Refused {}

fn refused<T>(text: impl Into<String>) -> Result<T, Refused> {
    Err(Refused(text.into()))
}

/// The download folder of `version`'s release: `https://github.com/<REPO>/releases/download/v<version>/`.
#[must_use]
pub fn release_base(version: &str) -> String {
    format!("https://github.com/{REPO}/releases/download/v{version}/")
}

/// `version` is a plain `X.Y.Z` (no pre-release, no build metadata, no `v`).
///
/// # Errors
/// [`Refused`] naming the version.
pub fn plain_version(version: &str) -> Result<(), Refused> {
    match semver::Version::parse(version) {
        Ok(parsed)
            if parsed.pre.is_empty()
                && parsed.build.is_empty()
                && parsed.to_string() == version =>
        {
            Ok(())
        }
        _ => refused(format!("version {version:?} is not a plain X.Y.Z")),
    }
}

/// The `version` of a Tauri config file's text (`app/tauri.conf.json`).
///
/// # Errors
/// [`Refused`] when the text is not JSON or has no string `version`.
pub fn app_version(config_text: &str) -> Result<String, Refused> {
    let config: Value = serde_json::from_str(config_text)
        .map_err(|e| Refused(format!("the app config is not JSON: {e}")))?;
    match config.get("version") {
        Some(Value::String(version)) => Ok(version.clone()),
        _ => refused("the app config has no \"version\""),
    }
}

/// The `pubkey` of an update channel file's text (the `plugins.updater` object the builds embed).
///
/// # Errors
/// [`Refused`] when the text is not JSON or has no string `pubkey`.
pub fn channel_pubkey(channel_text: &str) -> Result<String, Refused> {
    let channel: Value = serde_json::from_str(channel_text)
        .map_err(|e| Refused(format!("the channel file is not JSON: {e}")))?;
    match channel.get("pubkey") {
        Some(Value::String(key)) if !key.trim().is_empty() => Ok(key.trim().to_owned()),
        _ => refused("the channel file has no \"pubkey\""),
    }
}

fn base64_text(input: &str, what: &str) -> Result<String, Refused> {
    let bytes = STANDARD
        .decode(input.trim())
        .map_err(|_| Refused(format!("the {what} is not valid base64")))?;
    String::from_utf8(bytes).map_err(|_| Refused(format!("the {what} is not text")))
}

fn same_version(signed: &str, announced: &str) -> bool {
    let signed = signed.trim_start_matches('v');
    let announced = announced.trim_start_matches('v');
    match (
        semver::Version::parse(signed),
        semver::Version::parse(announced),
    ) {
        (Ok(a), Ok(b)) => a == b,
        _ => signed == announced,
    }
}

/// Checks one update signature the way the app does, plus the file binding.
///
/// `signature` and `pubkey` are base64 of the minisign texts (as the Tauri CLI writes them). The signature must
/// verify over `data` (prehashed only), and its trusted comment (`timestamp:..\tfile:..\tversion:..`) must name
/// `version:<version>` (semver-equal, like the app) and, when `file_name` is given, `file:<file_name>` exactly.
///
/// # Errors
/// [`Refused`] with the failing rule.
pub fn verify_signature(
    data: &[u8],
    signature: &str,
    pubkey: &str,
    version: &str,
    file_name: Option<&str>,
) -> Result<(), Refused> {
    let key = PublicKey::decode(&base64_text(pubkey, "public key")?)
        .map_err(|_| Refused("the public key does not decode".to_owned()))?;
    let signature = Signature::decode(&base64_text(signature, "signature")?)
        .map_err(|_| Refused("the signature does not decode".to_owned()))?;
    key.verify(data, &signature, false).map_err(|_| {
        Refused("the signature does not verify with the release public key".to_owned())
    })?;
    // Only after the signature verified is the trusted comment trustworthy.
    let comment = signature.trusted_comment();
    let field = |name: &str| {
        comment
            .split('\t')
            .find_map(|pair| pair.strip_prefix(name))
            .map(str::to_owned)
    };
    match field("version:") {
        Some(signed) if same_version(&signed, version) => {}
        Some(signed) => return refused(format!("signed for version {signed}, not {version}")),
        None => return refused("the signature carries no version (sign with --app-version)"),
    }
    if let Some(wanted) = file_name {
        match field("file:") {
            Some(signed) if signed == wanted => {}
            Some(signed) => return refused(format!("signed as file {signed:?}, not {wanted:?}")),
            None => return refused("the signature names no file"),
        }
    }
    Ok(())
}

fn name_rules(name: &str, kind: Kind, version: &str) -> Result<(), Refused> {
    if name.is_empty()
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    {
        return refused(format!(
            "asset name {name:?} has characters GitHub would rewrite (only A-Z a-z 0-9 . _ - are kept)"
        ));
    }
    if !name.ends_with(kind.suffix()) {
        return refused(format!("asset {name:?} is not a {} file", kind.suffix()));
    }
    if !name.contains(&format!("_{version}_")) {
        return refused(format!(
            "asset name {name:?} does not carry _{version}_ (another build?)"
        ));
    }
    Ok(())
}

fn read_artifact(path: &Path, kind: Kind) -> Result<Vec<u8>, Refused> {
    let data = std::fs::read(path).map_err(|e| Refused(format!("{}: {e}", path.display())))?;
    if !data.starts_with(kind.magic()) {
        return refused(format!(
            "{} does not start like a {} file",
            path.display(),
            kind.suffix()
        ));
    }
    Ok(data)
}

fn read_signature(artifact: &Path) -> Result<String, Refused> {
    let mut sig = artifact.as_os_str().to_owned();
    sig.push(".sig");
    let sig = PathBuf::from(sig);
    match std::fs::read_to_string(&sig) {
        Ok(text) if !text.trim().is_empty() => Ok(text.trim().to_owned()),
        Ok(_) => refused(format!("{} is empty", sig.display())),
        Err(e) => refused(format!("{}: {e}", sig.display())),
    }
}

/// One platform's artifact found in the release folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    /// The manifest key (`windows-x86_64`, ...).
    pub platform: &'static str,
    /// The file name (= the uploaded asset's name).
    pub name: String,
    /// The file.
    pub path: PathBuf,
}

/// Finds exactly one artifact per platform in `dir` (by name suffix; `.sig` files and other files are ignored).
///
/// # Errors
/// [`Refused`] when the folder cannot be read, a platform has none, or a platform has more than one.
pub fn find_assets(dir: &Path) -> Result<Vec<Asset>, Refused> {
    let entries = std::fs::read_dir(dir).map_err(|e| Refused(format!("{}: {e}", dir.display())))?;
    let mut found: BTreeMap<&'static str, Vec<(String, PathBuf)>> = BTreeMap::new();
    for entry in entries {
        let entry = entry.map_err(|e| Refused(format!("{}: {e}", dir.display())))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let is_file = entry.file_type().is_ok_and(|t| t.is_file());
        for (platform, kind) in PLATFORMS {
            if is_file && name.ends_with(kind.suffix()) {
                found
                    .entry(platform)
                    .or_default()
                    .push((name.clone(), entry.path()));
            }
        }
    }
    let mut assets = Vec::new();
    for (platform, kind) in PLATFORMS {
        match found.remove(platform).unwrap_or_default().as_slice() {
            [] => {
                return refused(format!(
                    "no {} for {platform} in {}",
                    kind.suffix(),
                    dir.display()
                ));
            }
            [(name, path)] => assets.push(Asset {
                platform,
                name: name.clone(),
                path: path.clone(),
            }),
            many => {
                let names: Vec<&str> = many.iter().map(|(n, _)| n.as_str()).collect();
                return refused(format!(
                    "more than one {} for {platform}: {names:?}",
                    kind.suffix()
                ));
            }
        }
    }
    Ok(assets)
}

/// The field of the oldest system version (macOS, the Windows build); the app reads the same name.
pub const MINIMUM_SYSTEM_VERSION: &str = "minimumSystemVersion";

/// The field of the oldest glibc (Linux); the app reads the same name.
pub const MINIMUM_GLIBC: &str = "minimumGlibc";

/// The oldest systems the release's packages run on, as the release step reads them from the artifacts; `None` =
/// that entry carries no minimum.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Minimums<'a> {
    /// `darwin-aarch64` `minimumSystemVersion`: the macOS bundle's `LSMinimumSystemVersion` (`11.0`).
    pub macos: Option<&'a str>,
    /// `windows-x86_64` `minimumSystemVersion`: the oldest Windows build (`10.0.17763`).
    pub windows: Option<&'a str>,
    /// `linux-x86_64` `minimumGlibc`: the highest glibc symbol version the `AppImage`'s programs need (`2.39`).
    pub linux_glibc: Option<&'a str>,
}

impl<'a> Minimums<'a> {
    /// The field and value `platform`'s entry carries, `None` when it carries none.
    #[must_use]
    pub fn of(&self, platform: &str) -> Option<(&'static str, &'a str)> {
        match platform {
            "darwin-aarch64" => self.macos.map(|v| (MINIMUM_SYSTEM_VERSION, v)),
            "windows-x86_64" => self.windows.map(|v| (MINIMUM_SYSTEM_VERSION, v)),
            "linux-x86_64" => self.linux_glibc.map(|v| (MINIMUM_GLIBC, v)),
            _ => None,
        }
    }

    fn check_values(&self) -> Result<(), Refused> {
        for value in [self.macos, self.windows, self.linux_glibc]
            .into_iter()
            .flatten()
        {
            system_version_text(value)?;
        }
        Ok(())
    }
}

/// `version` is a system version the app can compare: dotted numbers, at most four parts (`11.0`, `14`, `11.7.10`,
/// `10.0.17763`, the glibc `2.39`).
///
/// # Errors
/// [`Refused`] naming the value.
pub fn system_version_text(version: &str) -> Result<(), Refused> {
    let parts: Vec<&str> = version.split('.').collect();
    if parts.len() <= 4
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.len() <= 9 && p.bytes().all(|b| b.is_ascii_digit()))
    {
        Ok(())
    } else {
        refused(format!("{version:?} is not a system version (like 11.0)"))
    }
}

/// The manifest text for already-checked parts (no checks here: [`build`] and the format tests use it).
/// `entries`: `(platform key, url, signature)`; `minimums`: each entry's oldest system ([`Minimums::of`]).
#[must_use]
pub fn render(
    version: &str,
    notes: &str,
    pub_date: &str,
    entries: &[(&str, &str, &str)],
    minimums: &Minimums<'_>,
) -> String {
    let mut platforms = Map::new();
    for (key, url, signature) in entries {
        let mut entry = Map::new();
        entry.insert(
            "signature".to_owned(),
            Value::String((*signature).to_owned()),
        );
        entry.insert("url".to_owned(), Value::String((*url).to_owned()));
        if let Some((field, minimum)) = minimums.of(key) {
            entry.insert(field.to_owned(), Value::String(minimum.to_owned()));
        }
        platforms.insert((*key).to_owned(), Value::Object(entry));
    }
    let mut manifest = Map::new();
    manifest.insert("version".to_owned(), Value::String(version.to_owned()));
    manifest.insert("notes".to_owned(), Value::String(notes.to_owned()));
    manifest.insert("pub_date".to_owned(), Value::String(pub_date.to_owned()));
    manifest.insert("platforms".to_owned(), Value::Object(platforms));
    let mut text = serde_json::to_string_pretty(&Value::Object(manifest)).unwrap_or_default();
    text.push('\n');
    text
}

/// What one release's manifest is built from.
#[derive(Debug, Clone)]
pub struct Release<'a> {
    /// The release version (`X.Y.Z`), already checked against the app config by the caller.
    pub version: &'a str,
    /// Release notes shown in the app's update notice.
    pub notes: &'a str,
    /// `YYYY-MM-DDTHH:MM:SSZ`.
    pub pub_date: &'a str,
    /// The release public key (base64 minisign text, the channel's `pubkey`).
    pub pubkey: &'a str,
    /// The oldest systems the packages run on, each written into its own entry.
    pub minimums: Minimums<'a>,
}

/// Builds the manifest text from the artifacts and `.sig` files in `dir`, and checks it with [`check`].
///
/// # Errors
/// [`Refused`] on the first broken rule (see the module docs).
pub fn build(release: &Release<'_>, dir: &Path) -> Result<String, Refused> {
    plain_version(release.version)?;
    release.minimums.check_values()?;
    let base = release_base(release.version);
    let mut entries = Vec::new();
    for asset in find_assets(dir)? {
        let kind = kind_of(asset.platform);
        name_rules(&asset.name, kind, release.version)?;
        entries.push((
            asset.platform,
            format!("{base}{}", asset.name),
            read_signature(&asset.path)?,
        ));
    }
    let borrowed: Vec<(&str, &str, &str)> = entries
        .iter()
        .map(|(k, u, s)| (*k, u.as_str(), s.as_str()))
        .collect();
    let text = render(
        release.version,
        release.notes,
        release.pub_date,
        &borrowed,
        &release.minimums,
    );
    check(
        &text,
        dir,
        release.version,
        release.pubkey,
        &release.minimums,
    )?;
    Ok(text)
}

fn kind_of(platform: &str) -> Kind {
    PLATFORMS
        .iter()
        .find(|(key, _)| *key == platform)
        .map_or(Kind::AppImage, |(_, kind)| *kind)
}

fn text_field<'a>(object: &'a Map<String, Value>, key: &str) -> Result<&'a str, Refused> {
    match object.get(key) {
        Some(Value::String(text)) => Ok(text),
        _ => refused(format!("\"{key}\" is missing or not a text")),
    }
}

/// Checks a manifest text against the artifacts in `dir` (see the module docs for every rule); `minimums` are the
/// entries' expected oldest systems (`None`: that entry has none).
///
/// # Errors
/// [`Refused`] on the first broken rule.
pub fn check(
    text: &str,
    dir: &Path,
    version: &str,
    pubkey: &str,
    minimums: &Minimums<'_>,
) -> Result<(), Refused> {
    plain_version(version)?;
    let manifest: Value = serde_json::from_str(text)
        .map_err(|e| Refused(format!("the manifest is not JSON: {e}")))?;
    let Some(manifest) = manifest.as_object() else {
        return refused("the manifest is not a JSON object");
    };
    for key in manifest.keys() {
        if !matches!(key.as_str(), "version" | "notes" | "pub_date" | "platforms") {
            return refused(format!("unexpected field {key:?}"));
        }
    }
    let announced = text_field(manifest, "version")?;
    if announced != version {
        return refused(format!(
            "the manifest announces {announced}, the release is {version}"
        ));
    }
    if text_field(manifest, "notes")?.trim().is_empty() {
        return refused("the release notes are empty");
    }
    let pub_date = text_field(manifest, "pub_date")?;
    if !valid_pub_date(pub_date) {
        return refused(format!("pub_date {pub_date:?} is not YYYY-MM-DDTHH:MM:SSZ"));
    }
    let Some(Value::Object(platforms)) = manifest.get("platforms") else {
        return refused("\"platforms\" is missing or not an object");
    };
    for key in platforms.keys() {
        if !PLATFORMS.iter().any(|(p, _)| p == key) {
            return refused(format!("unexpected platform {key:?}"));
        }
    }
    let base = release_base(version);
    for (platform, kind) in PLATFORMS {
        let Some(Value::Object(entry)) = platforms.get(platform) else {
            return refused(format!("platform {platform} is missing"));
        };
        let expected = minimums.of(platform);
        for field in [MINIMUM_SYSTEM_VERSION, MINIMUM_GLIBC] {
            let want = expected.filter(|(f, _)| *f == field).map(|(_, v)| v);
            let found = entry.get(field).map(Value::as_str);
            if found != want.map(Some) {
                return refused(format!(
                    "{platform}: \"{field}\" is {found:?}, expected {want:?}"
                ));
            }
        }
        if entry.len() != 2 + usize::from(expected.is_some()) {
            return refused(format!(
                "{platform}: only \"url\", \"signature\" and the platform's minimum are allowed"
            ));
        }
        let url = text_field(entry, "url")?;
        let Some(name) = url.strip_prefix(&base) else {
            return refused(format!("{platform}: url {url:?} is not under {base}"));
        };
        name_rules(name, kind, version).map_err(|r| Refused(format!("{platform}: {r}")))?;
        let path = dir.join(name);
        let data = read_artifact(&path, kind)?;
        let signature = text_field(entry, "signature")?;
        if signature != read_signature(&path)? {
            return refused(format!("{platform}: the signature differs from {name}.sig"));
        }
        verify_signature(&data, signature, pubkey, version, Some(name))
            .map_err(|r| Refused(format!("{platform} ({name}): {r}")))?;
    }
    Ok(())
}
