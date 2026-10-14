//! Release manifest generation and verification using assets and signing keys made in each test.
//! PREDICT: complete signed assets produce a usable manifest; malformed or mismatched inputs fail closed.

mod common;
mod update_manifest_rig;

use update_manifest_rig::{content, flow_from_manifest, names};

use std::io::Cursor;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use common::update_rig::{DirGuard, unique_dir};
use make_latest_json::{Refused, WriteRequest};
use serde_json::Value;

const VERSION: &str = "1.1.0";
const NOTES: &str = "Faster start.";

struct Key {
    pk: minisign::PublicKey,
    sk: minisign::SecretKey,
}

impl Key {
    fn new() -> Self {
        let pair = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        Self {
            pk: pair.pk,
            sk: pair.sk,
        }
    }
    fn pubkey(&self) -> String {
        STANDARD.encode(self.pk.to_box().unwrap().to_string())
    }
    /// Like `tauri signer sign --app-version <version>` on a file named `file`.
    fn sign(&self, data: &[u8], file: &str, version: &str) -> String {
        self.sign_comment(
            data,
            &format!("timestamp:1\tfile:{file}\tversion:{version}"),
        )
    }
    fn sign_comment(&self, data: &[u8], comment: &str) -> String {
        let signature = minisign::sign(
            Some(&self.pk),
            &self.sk,
            Cursor::new(data),
            Some(comment),
            Some("t"),
        )
        .unwrap();
        STANDARD.encode(signature.to_string())
    }
}

/// A release folder (`assets/`) plus the side files (app config, channel, public key, notes) next to it.
struct Release {
    _guard: DirGuard,
    root: PathBuf,
    assets: PathBuf,
    key: Key,
    version: String,
    pub_date: Option<String>,
}

impl Release {
    fn new(name: &str) -> Self {
        let root = unique_dir(&format!("manifest-{name}"));
        let assets = root.join("assets");
        std::fs::create_dir_all(&assets).unwrap();
        let release = Self {
            _guard: DirGuard(root.clone()),
            root,
            assets,
            key: Key::new(),
            version: VERSION.to_owned(),
            pub_date: Some("2026-10-12T09:00:00Z".to_owned()),
        };
        for asset in names(VERSION) {
            release.put(&asset, &content(&asset));
        }
        release.side_files(VERSION, &release.key.pubkey(), NOTES);
        release
    }

    fn side_files(&self, app_version: &str, channel_key: &str, notes: &str) {
        let config = serde_json::json!({ "productName": "Vibe Work", "version": app_version });
        std::fs::write(self.root.join("tauri.conf.json"), config.to_string()).unwrap();
        let channel = serde_json::json!({
            "pubkey": channel_key,
            "endpoints": ["https://github.com/martinsbrezauckis/vibe-work/releases/latest/download/latest.json"],
            "requireSignedVersion": true,
        });
        std::fs::write(self.root.join("channel.json"), channel.to_string()).unwrap();
        std::fs::write(self.root.join("updater.pub"), self.key.pubkey()).unwrap();
        std::fs::write(self.root.join("notes.txt"), notes).unwrap();
    }

    /// Writes an asset and its signature (made by this release's key, for its name and version).
    fn put(&self, name: &str, data: &[u8]) {
        std::fs::write(self.assets.join(name), data).unwrap();
        self.put_sig(name, &self.key.sign(data, name, VERSION));
    }

    fn put_sig(&self, name: &str, sig: &str) {
        std::fs::write(self.assets.join(format!("{name}.sig")), sig).unwrap();
    }

    fn out(&self) -> PathBuf {
        self.assets.join("latest.json")
    }

    fn write(&self) -> Result<PathBuf, Refused> {
        let (config, channel, pubkey, notes) = (
            self.root.join("tauri.conf.json"),
            self.root.join("channel.json"),
            self.root.join("updater.pub"),
            self.root.join("notes.txt"),
        );
        let out = self.out();
        make_latest_json::write_manifest(&WriteRequest {
            version: &self.version,
            app_config: &config,
            channel: &channel,
            pubkey: &pubkey,
            notes: &notes,
            pub_date: self.pub_date.as_deref(),
            artifacts: &self.assets,
            out: &out,
        })
    }

    fn check(&self, manifest: &Path) -> Result<(), Refused> {
        make_latest_json::check_manifest(
            &self.version,
            &self.root.join("tauri.conf.json"),
            &self.root.join("updater.pub"),
            &self.assets,
            manifest,
        )
    }
}

fn refused_with(result: Result<PathBuf, Refused>, words: &str, release: &Release) {
    match result {
        Ok(path) => panic!(
            "expected a refusal containing {words:?}, got a manifest at {}",
            path.display()
        ),
        Err(Refused(text)) => assert!(
            text.contains(words),
            "refusal {text:?} does not contain {words:?}"
        ),
    }
    assert!(
        !release.out().exists(),
        "nothing may be written on a refusal"
    );
    assert!(!release.assets.join("latest.json.tmp").exists());
}

// ---- ----------------------------------------------------------------------------------------------------

#[test]
fn complete_release_gives_the_pinned_manifest() {
    let release = Release::new("g1");
    let out = release.write().unwrap();
    let manifest: Value = serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
    assert_eq!(manifest["version"], VERSION);
    assert_eq!(manifest["notes"], NOTES);
    assert_eq!(manifest["pub_date"], "2026-10-12T09:00:00Z");
    let platforms = manifest["platforms"].as_object().unwrap();
    let keys: Vec<&str> = platforms.keys().map(String::as_str).collect();
    assert_eq!(keys.len(), 3);
    for (key, name) in ["windows-x86_64", "darwin-aarch64", "linux-x86_64"]
        .iter()
        .zip(names(VERSION))
    {
        let entry = &platforms[*key];
        assert_eq!(
            entry["url"],
            format!(
                "https://github.com/martinsbrezauckis/vibe-work/releases/download/v{VERSION}/{name}"
            )
        );
        let sig = std::fs::read_to_string(release.assets.join(format!("{name}.sig"))).unwrap();
        assert_eq!(entry["signature"], sig.trim());
    }
    release.check(&out).unwrap();
}

// ---- write refusals --------------------------------------------------------------------------------------

#[test]
fn release_version_must_equal_the_app_version() {
    let release = Release::new("appver");
    release.side_files("1.0.9", &release.key.pubkey(), NOTES);
    refused_with(
        release.write(),
        "differs from the app's version 1.0.9",
        &release,
    );
}

#[test]
fn a_signature_for_another_version_is_refused() {
    let release = Release::new("sigver");
    let [exe, _, _] = names(VERSION);
    release.put_sig(&exe, &release.key.sign(&content(&exe), &exe, "9.9.9"));
    refused_with(release.write(), "signed for version 9.9.9", &release);
}

#[test]
fn a_signature_without_a_version_is_refused() {
    let release = Release::new("sigver0");
    let [exe, _, _] = names(VERSION);
    release.put_sig(
        &exe,
        &release
            .key
            .sign_comment(&content(&exe), &format!("timestamp:1\tfile:{exe}")),
    );
    refused_with(release.write(), "carries no version", &release);
}

#[test]
fn a_missing_platform_is_refused() {
    let release = Release::new("missing");
    let [_, _, appimage] = names(VERSION);
    std::fs::remove_file(release.assets.join(&appimage)).unwrap();
    refused_with(release.write(), "no .AppImage for linux-x86_64", &release);
}

#[test]
fn two_candidates_for_one_platform_are_refused() {
    let release = Release::new("twice");
    release.put(
        &format!("Vibe-Work_{VERSION}_x64-old-setup.exe"),
        &content("x.exe"),
    );
    refused_with(release.write(), "more than one -setup.exe", &release);
}

#[test]
fn another_key_is_refused() {
    let release = Release::new("otherkey");
    let [_, _, appimage] = names(VERSION);
    release.put_sig(
        &appimage,
        &Key::new().sign(&content(&appimage), &appimage, VERSION),
    );
    refused_with(release.write(), "does not verify", &release);
}

#[test]
fn a_tampered_artifact_is_refused() {
    let release = Release::new("tamper");
    let [_, archive, _] = names(VERSION);
    let mut data = content(&archive);
    data.push(b'!');
    std::fs::write(release.assets.join(&archive), data).unwrap();
    refused_with(release.write(), "does not verify", &release);
}

#[test]
fn asset_names_github_would_rewrite_are_refused() {
    let release = Release::new("space");
    let [_, _, appimage] = names(VERSION);
    std::fs::remove_file(release.assets.join(&appimage)).unwrap();
    std::fs::remove_file(release.assets.join(format!("{appimage}.sig"))).unwrap();
    let spaced = format!("Vibe Work_{VERSION}_amd64.AppImage");
    release.put(&spaced, &content(&spaced));
    refused_with(release.write(), "characters GitHub would rewrite", &release);
}

#[test]
fn a_name_without_the_version_is_refused() {
    let release = Release::new("noversion");
    let [_, _, appimage] = names(VERSION);
    std::fs::remove_file(release.assets.join(&appimage)).unwrap();
    release.put("Vibe-Work_1.0.0_amd64.AppImage", &content("x.AppImage"));
    refused_with(release.write(), "does not carry _1.1.0_", &release);
}

#[test]
fn a_wrong_artifact_kind_is_refused() {
    let release = Release::new("kind");
    let [exe, _, _] = names(VERSION);
    release.put(&exe, b"PK\x03\x04 a zip, not the installer");
    refused_with(
        release.write(),
        "does not start like a -setup.exe file",
        &release,
    );
}

#[test]
fn the_channel_key_must_be_the_signing_key() {
    let release = Release::new("channel");
    release.side_files(VERSION, &Key::new().pubkey(), NOTES);
    refused_with(release.write(), "channel's pubkey", &release);
}

#[test]
fn empty_notes_and_bad_dates_are_refused() {
    let release = Release::new("notes");
    release.side_files(VERSION, &release.key.pubkey(), "  \n");
    refused_with(release.write(), "release notes are empty", &release);
    let mut release = Release::new("date");
    release.pub_date = Some("2026-10-12 09:00".to_owned());
    refused_with(release.write(), "pub_date", &release);
}

#[test]
fn a_signature_for_another_file_is_refused() {
    let release = Release::new("file");
    let [exe, archive, _] = names(VERSION);
    release.put_sig(
        &archive,
        &release.key.sign(&content(&archive), &exe, VERSION),
    );
    refused_with(release.write(), "signed as file", &release);
}

#[test]
fn a_pre_release_version_is_refused() {
    let mut release = Release::new("pre");
    release.version = "1.1.0-rc.1".to_owned();
    release.side_files("1.1.0-rc.1", &release.key.pubkey(), NOTES);
    refused_with(release.write(), "not a plain X.Y.Z", &release);
}

// ---- check of an edited manifest -------------------------------------------------------------------------

/// One edit of a written manifest.
type Edit<'a> = Box<dyn Fn(&mut Value) + 'a>;

#[test]
fn check_refuses_edited_manifests() {
    let release = Release::new("edits");
    let out = release.write().unwrap();
    let good: Value = serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
    let [exe, _, _] = names(VERSION);
    let pinned = format!(
        "https://github.com/martinsbrezauckis/vibe-work/releases/download/v{VERSION}/{exe}"
    );
    let cases: Vec<(&str, Edit<'_>, &str)> = vec![
        (
            "other repo",
            Box::new(|m: &mut Value| {
                m["platforms"]["windows-x86_64"]["url"] = pinned
                    .replace("martinsbrezauckis/vibe-work", "someone/vibe-work")
                    .into();
            }),
            "is not under",
        ),
        (
            "other tag",
            Box::new(|m: &mut Value| {
                m["platforms"]["windows-x86_64"]["url"] =
                    pinned.replace(&format!("/v{VERSION}/"), "/v1.0.0/").into();
            }),
            "is not under",
        ),
        (
            "http",
            Box::new(|m: &mut Value| {
                m["platforms"]["windows-x86_64"]["url"] =
                    pinned.replace("https://", "http://").into();
            }),
            "is not under",
        ),
        (
            "path escape",
            Box::new(|m: &mut Value| {
                m["platforms"]["windows-x86_64"]["url"] = format!("{pinned}/../../evil.exe").into();
            }),
            "characters GitHub would rewrite",
        ),
        (
            "extra platform",
            Box::new(|m: &mut Value| {
                m["platforms"]["darwin-x86_64"] = m["platforms"]["darwin-aarch64"].clone();
            }),
            "unexpected platform",
        ),
        (
            "extra field",
            Box::new(|m: &mut Value| {
                m["releasePage"] = "https://example.com".into();
            }),
            "unexpected field",
        ),
        (
            "edited version",
            Box::new(|m: &mut Value| {
                m["version"] = "1.2.0".into();
            }),
            "announces 1.2.0",
        ),
        (
            "swapped signature",
            Box::new(|m: &mut Value| {
                m["platforms"]["windows-x86_64"]["signature"] =
                    m["platforms"]["linux-x86_64"]["signature"].clone();
            }),
            "differs from",
        ),
    ];
    for (label, edit, words) in cases {
        let mut manifest = good.clone();
        edit(&mut manifest);
        let path = release
            .root
            .join(format!("edited-{}.json", label.replace(' ', "-")));
        std::fs::write(&path, manifest.to_string()).unwrap();
        match release.check(&path) {
            Ok(()) => panic!("{label}: accepted"),
            Err(Refused(text)) => {
                assert!(text.contains(words), "{label}: {text:?} lacks {words:?}")
            }
        }
    }
    release.check(&out).unwrap();
}

// ---- ------------------------------------------------------------------------------------------------------

#[test]
fn the_tool_and_the_app_agree_on_signatures() {
    let key = Key::new();
    let other = Key::new();
    let data = b"MZ an installer".to_vec();
    let mut tampered = data.clone();
    tampered.push(0);
    let cases = [
        (
            "good",
            data.clone(),
            key.sign(&data, "a.exe", VERSION),
            true,
        ),
        (
            "v-prefixed version",
            data.clone(),
            key.sign(&data, "a.exe", "v1.1.0"),
            true,
        ),
        (
            "other key",
            data.clone(),
            other.sign(&data, "a.exe", VERSION),
            false,
        ),
        (
            "tampered",
            tampered,
            key.sign(&data, "a.exe", VERSION),
            false,
        ),
        (
            "other version",
            data.clone(),
            key.sign(&data, "a.exe", "1.0.0"),
            false,
        ),
        (
            "no version",
            data.clone(),
            key.sign_comment(&data, "timestamp:1\tfile:a.exe"),
            false,
        ),
        ("not base64", data.clone(), "%%%".to_owned(), false),
    ];
    for (label, bytes, sig, expected) in cases {
        let tool =
            make_latest_json::verify_signature(&bytes, &sig, &key.pubkey(), VERSION, None).is_ok();
        let app =
            vw_host::update_verify::verify_package(&bytes, &sig, &key.pubkey(), VERSION).is_ok();
        assert_eq!(
            (tool, app),
            (expected, expected),
            "{label}: tool {tool}, app {app}"
        );
    }
}

// ---- ------------------------------------------------------------------------------------------------------

/// The app's update flow fed from a written `latest.json`: the `linux-x86_64` entry is the announcement, the asset
/// its URL names (read from `assets`) the download. Returns (reached ready, package left on disk).
#[tokio::test]
async fn the_manifest_drives_the_app_update_flow() {
    let release = Release::new("g4a");
    let out = release.write().unwrap();
    assert_eq!(
        flow_from_manifest(&out, &release.assets, &release.key.pubkey(), false, VERSION).await,
        (true, true)
    );
    assert_eq!(
        flow_from_manifest(&out, &release.assets, &release.key.pubkey(), true, VERSION).await,
        (false, false),
        "a changed download never reaches ready and leaves nothing"
    );
}
