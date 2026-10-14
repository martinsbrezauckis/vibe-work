//! Signature check for an update package: the same minisign rules the updater plugin applies right
//! after its own download, repeated by us whenever a package comes back from disk and before anything is called
//! "ready". The plugin's `install` does not verify, so a package that lay on disk across a restart is trusted
//! only through this function.
//!
//! Rules (the plugin's `verify_signature`, `tauri-plugin-updater` 2.13.2, plus `requireSignedVersion` which is
//! mandatory in every channel we accept, see [`crate::updates::channel`]):
//! - `pubkey` and `signature` are base64 of the minisign text files, exactly as the Tauri CLI writes them;
//! - the Ed25519 signature must verify over the package bytes;
//! - the signature's trusted comment (covered by minisign's global signature) must carry `version:<v>` equal to the
//!   announced version, compared as semver when both parse (so `v1.2.3` equals `1.2.3`), literally otherwise. A
//!   signature without a version is refused: it could be an old release replayed as the new one.
//!
//! Depends on: minisign-verify, base64. Called by: [`crate::update_flow`], [`crate::update_store`].

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use minisign_verify::{PublicKey, Signature};

fn base64_text(input: &str) -> Result<String, String> {
    let bytes = STANDARD
        .decode(input.trim())
        .map_err(|_| "the key or signature is not valid base64".to_owned())?;
    String::from_utf8(bytes).map_err(|_| "the key or signature is not text".to_owned())
}

/// The `version:` value of a Tauri-written trusted comment (`timestamp:..\tfile:..\tversion:1.2.3`).
fn signed_version(trusted_comment: &str) -> Option<&str> {
    trusted_comment
        .split('\t')
        .find_map(|pair| pair.strip_prefix("version:"))
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

/// Checks `package` against the channel's `pubkey` and the announced `signature` for `version`.
///
/// # Errors
/// A short plain-words reason: bad key or signature encoding, signature does not verify, signed version missing
/// or different from the announced one.
pub fn verify_package(
    package: &[u8],
    signature: &str,
    pubkey: &str,
    version: &str,
) -> Result<(), String> {
    let key = PublicKey::decode(&base64_text(pubkey)?)
        .map_err(|_| "the update key in this build is not valid".to_owned())?;
    let signature = Signature::decode(&base64_text(signature)?)
        .map_err(|_| "the update signature is not valid".to_owned())?;
    key.verify(package, &signature, true)
        .map_err(|_| "the update's signature does not match this build's key".to_owned())?;
    // Only after the signature verified is the trusted comment trustworthy.
    match signed_version(signature.trusted_comment()) {
        Some(signed) if same_version(signed, version) => Ok(()),
        Some(signed) => Err(format!(
            "the update was signed for version {signed} but announced as {version}"
        )),
        None => Err("the update's signature carries no version".to_owned()),
    }
}
