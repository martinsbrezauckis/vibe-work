//! Per-user TPM wrapping for stored vault and Chrome-owner secrets. Used by vw-vault on Windows.
//! No installer service, Hello enrollment or new sign-in is required. The private RSA key is non-exportable.
//! This is device/user binding, NOT an app boundary: another process with the same user token can invoke CNG.
//! Depends on: Windows CNG and device/user identity.
//! Called by: vault stored-secret wrapping.
use windows::Win32::Foundation::{
    NTE_BAD_KEYSET, NTE_DEVICE_NOT_FOUND, NTE_DEVICE_NOT_READY, NTE_NOT_SUPPORTED,
    TBS_E_TPM_NOT_FOUND,
};
use windows::Win32::Security::Cryptography::{
    BCRYPT_OAEP_PADDING_INFO, BCRYPT_RSA_ALGORITHM, BCRYPT_SHA256_ALGORITHM, CERT_KEY_SPEC,
    MS_PLATFORM_CRYPTO_PROVIDER, NCRYPT_FLAGS, NCRYPT_HANDLE, NCRYPT_KEY_HANDLE,
    NCRYPT_LENGTH_PROPERTY, NCRYPT_PAD_OAEP_FLAG, NCRYPT_PROV_HANDLE, NCRYPT_SILENT_FLAG,
    NCryptCreatePersistedKey, NCryptDecrypt, NCryptEncrypt, NCryptFinalizeKey, NCryptFreeObject,
    NCryptOpenKey, NCryptOpenStorageProvider, NCryptSetProperty,
};
use windows::core::{Result, w};
use zeroize::Zeroizing;

const KEY_NAME: windows::core::PCWSTR = w!("vibe-work.vault-wrapping.v1");
struct Handle(NCRYPT_HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: this wrapper uniquely owns a successful CNG handle and releases it once.
        let _ = unsafe { NCryptFreeObject(self.0) };
    }
}
fn key(create: bool) -> Result<(Handle, Handle)> {
    let mut provider = NCRYPT_PROV_HANDLE::default();
    // SAFETY: outputs are live storage; the fixed provider name selects only the TPM provider, never software.
    unsafe {
        NCryptOpenStorageProvider(&raw mut provider, MS_PLATFORM_CRYPTO_PROVIDER, 0)?;
    }
    let provider_owner = Handle(NCRYPT_HANDLE(provider.0));
    let mut key = NCRYPT_KEY_HANDLE::default();
    // SAFETY: provider is live, and the fixed name is a terminated UTF-16 constant. No machine-key flag: per user.
    let opened = unsafe {
        NCryptOpenKey(
            provider,
            &raw mut key,
            KEY_NAME,
            CERT_KEY_SPEC(0),
            NCRYPT_SILENT_FLAG,
        )
    };
    if let Err(error) = opened {
        // Only a missing key permits creation. Never overwrite or rotate an existing key on an access failure.
        if !create || error.code() != NTE_BAD_KEYSET {
            return Err(error);
        }
        // SAFETY: same live provider and output. The KSP defaults to no private-key export; we never enable export.
        unsafe {
            NCryptCreatePersistedKey(
                provider,
                &raw mut key,
                BCRYPT_RSA_ALGORITHM,
                KEY_NAME,
                CERT_KEY_SPEC(0),
                NCRYPT_FLAGS(0),
            )?;
        }
        let owner = Handle(NCRYPT_HANDLE(key.0));
        // SAFETY: key is live and not finalized. Length is a native DWORD. Silent forbids enrollment/UI prompts.
        unsafe {
            NCryptSetProperty(
                owner.0,
                NCRYPT_LENGTH_PROPERTY,
                &2048_u32.to_le_bytes(),
                NCRYPT_FLAGS(0),
            )?;
            NCryptFinalizeKey(key, NCRYPT_SILENT_FLAG)?;
        }
        return Ok((provider_owner, owner));
    }
    Ok((provider_owner, Handle(NCRYPT_HANDLE(key.0))))
}
fn crypt(key: &Handle, input: &[u8], encrypt: bool) -> Result<Zeroizing<Vec<u8>>> {
    let padding = BCRYPT_OAEP_PADDING_INFO {
        pszAlgId: BCRYPT_SHA256_ALGORITHM,
        pbLabel: std::ptr::null_mut(),
        cbLabel: 0,
    };
    let flags = NCRYPT_PAD_OAEP_FLAG | NCRYPT_SILENT_FLAG;
    let mut size = 0;
    let mut call = |output: Option<&mut [u8]>| {
        // SAFETY: CNG gets live input/output slices and a correctly typed OAEP descriptor for the call's duration.
        unsafe {
            if encrypt {
                NCryptEncrypt(
                    NCRYPT_KEY_HANDLE(key.0.0),
                    Some(input),
                    Some((&raw const padding).cast()),
                    output,
                    &raw mut size,
                    flags,
                )
            } else {
                NCryptDecrypt(
                    NCRYPT_KEY_HANDLE(key.0.0),
                    Some(input),
                    Some((&raw const padding).cast()),
                    output,
                    &raw mut size,
                    flags,
                )
            }
        }
    };
    // RSA-2048 has a fixed upper output bound; use a single call to avoid retaining an output-length borrow.
    let mut output = Zeroizing::new(vec![0; 256]);
    call(Some(&mut output))?;
    output.truncate(size as usize);
    Ok(output)
}
fn error(error: &windows::core::Error) -> String {
    format!("TPM key operation failed ({})", error.code())
}

/// Wraps a random data key with a non-exportable per-user TPM RSA key.
/// Returns None only when this computer has no usable TPM provider. The caller retains its existing OS store.
/// # Errors
/// An existing TPM key is inaccessible, creation is refused, or encryption fails.
pub fn wrap(data: &[u8; 32]) -> std::result::Result<Option<Vec<u8>>, String> {
    let (_provider, key) = match key(true) {
        Ok(key) => key,
        Err(e)
            if matches!(
                e.code(),
                NTE_NOT_SUPPORTED
                    | NTE_DEVICE_NOT_READY
                    | NTE_DEVICE_NOT_FOUND
                    | TBS_E_TPM_NOT_FOUND
            ) =>
        {
            return Ok(None);
        }
        Err(e) => return Err(error(&e)),
    };
    crypt(&key, data, true)
        .map(|v| Some(v.to_vec()))
        .map_err(|e| error(&e))
}

/// Unwraps a data key. A missing/reset TPM is an error, never a request to create a replacement key.
/// # Errors
/// Missing key, unauthorized operation, corrupted ciphertext or unsupported TPM.
pub fn unwrap(wrapped: &[u8]) -> std::result::Result<Zeroizing<Vec<u8>>, String> {
    let (_provider, key) = key(false).map_err(|e| error(&e))?;
    crypt(&key, wrapped, false).map_err(|e| error(&e))
}
