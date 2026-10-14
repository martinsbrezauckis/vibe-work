//! Signs a release-channel file with the minisign updater key: the supported-Vibe manifest
//! `vibe-support.json`, next to `latest.json`. The output is `<file>.sig` holding base64 of the minisign signature
//! text, exactly the encoding the Tauri CLI writes for update packages, so the app checks both with one rule.
//!
//! The trusted comment (covered by the global signature) is `timestamp:<unix seconds>\tfile:<file name>`; the app
//! requires `file:vibe-support.json`, so a signature made for another file with the same key never passes as the
//! manifest's.
//!
//! **Secrets.** The private key (the Tauri key file text: base64 of the encrypted minisign secret key) and its
//! password are read from two inputs the caller gives as paths, meant to be pipes from a password manager (`--key <(…)`): never from
//! argv or the environment, never written anywhere, never printed. An empty input is refused (a failed password-manager call
//! yields one). Before anything is written the new signature is verified against the expected public key, so a
//! wrong key or password writes nothing.
//!
//! Depends on: minisign, base64. Called by: `src/main.rs`; the private tests (sign with a throwaway key, then the
//! app's verifier).

#![forbid(unsafe_code)]

use std::io::Cursor;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

/// Why signing did not happen. The texts never contain key material.
#[derive(Debug)]
pub enum SignError {
    /// An input is empty (a failed password-manager call, a missing pipe).
    Empty(&'static str),
    /// An input could not be read.
    Read(&'static str, std::io::Error),
    /// The private key does not decode, or the password does not open it.
    Key,
    /// The public key does not decode.
    PublicKey,
    /// The signature does not verify with the expected public key (another key pair).
    Mismatch,
    /// Signing or writing failed.
    Write(String),
}

impl std::fmt::Display for SignError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty(what) => write!(f, "the {what} input is empty: refused"),
            Self::Read(what, e) => write!(f, "the {what} input cannot be read: {e}"),
            Self::Key => f.write_str("the private key does not decode or the password is wrong"),
            Self::PublicKey => f.write_str("the expected public key does not decode"),
            Self::Mismatch => {
                f.write_str("the private key does not belong to the expected public key")
            }
            Self::Write(why) => write!(f, "signing failed: {why}"),
        }
    }
}

impl std::error::Error for SignError {}

fn read_input(path: &Path, what: &'static str) -> Result<String, SignError> {
    let text = std::fs::read_to_string(path).map_err(|e| SignError::Read(what, e))?;
    if text.trim().is_empty() {
        return Err(SignError::Empty(what));
    }
    Ok(text)
}

fn decoded(text: &str) -> Option<String> {
    let bytes = STANDARD.decode(text.trim()).ok()?;
    String::from_utf8(bytes).ok()
}

/// The signature text for `data` named `file_name`, as `<file>.sig` holds it (base64 of the minisign text).
///
/// # Errors
/// [`SignError::Key`], [`SignError::PublicKey`], [`SignError::Mismatch`], [`SignError::Write`].
pub fn sign_bytes(
    data: &[u8],
    file_name: &str,
    private_key: &str,
    password: &str,
    public_key: &str,
    timestamp: u64,
) -> Result<String, SignError> {
    let secret_box = decoded(private_key)
        .and_then(|text| minisign::SecretKeyBox::from_string(&text).ok())
        .ok_or(SignError::Key)?;
    // Some(..) always: `None` would make the crate prompt on the terminal.
    let secret = secret_box
        .into_secret_key(Some(password.to_owned()))
        .map_err(|_| SignError::Key)?;
    let public = decoded(public_key)
        .and_then(|text| minisign::PublicKeyBox::from_string(&text).ok())
        .and_then(|b| b.into_public_key().ok())
        .ok_or(SignError::PublicKey)?;
    let trusted = format!("timestamp:{timestamp}\tfile:{file_name}");
    let signature = minisign::sign(
        Some(&public),
        &secret,
        Cursor::new(data),
        Some(&trusted),
        Some("signature from the Vibe Work updater key"),
    )
    .map_err(|_| SignError::Mismatch)?;
    minisign::verify(&public, &signature, Cursor::new(data), true, false, false)
        .map_err(|_| SignError::Mismatch)?;
    Ok(STANDARD.encode(signature.to_string()))
}

/// Signs `file` into `<file>.sig` with the key read from `key_input` and the password from `password_input`
/// (paths: pipes from a password manager), after checking the result against `public_key` (base64 minisign text, the Tauri
/// config's `pubkey`). Returns the signature file's path.
///
/// # Errors
/// [`SignError`]; on any error nothing is written.
pub fn sign_file(
    file: &Path,
    key_input: &Path,
    password_input: &Path,
    public_key: &str,
) -> Result<PathBuf, SignError> {
    let data =
        std::fs::read(file).map_err(|e| SignError::Write(format!("{}: {e}", file.display())))?;
    let key = read_input(key_input, "private key")?;
    let password = read_input(password_input, "password")?;
    // A password manager's output ends with one newline; the password itself is the first line exactly.
    let password = password.lines().next().unwrap_or_default();
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| SignError::Write("the file has no name".to_owned()))?;
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let signature = sign_bytes(&data, &name, &key, password, public_key, timestamp)?;
    let out = file.with_file_name(format!("{name}.sig"));
    let temp = file.with_file_name(format!("{name}.sig.tmp"));
    std::fs::write(&temp, &signature)
        .and_then(|()| std::fs::rename(&temp, &out))
        .map_err(|e| SignError::Write(format!("{}: {e}", out.display())))?;
    Ok(out)
}
