//! OS-owned proxy/PAC resolution for the optional guard. Depends on `WinHTTP`, `CFNetwork` or libproxy.
//! Called on bounded worker threads after destination policy approval. Returns URL lists in OS order;
//! errors never include settings or credentials. No JavaScript evaluator or WPAD discovery is implemented here.
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
mod linux_log;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(target_os = "macos", test))]
mod macos_policy;
#[cfg(any(windows, target_os = "macos", test))]
mod manual;
#[cfg(windows)]
mod windows;
#[cfg(any(windows, test))]
mod windows_policy;

/// An exact, opaque snapshot of the OS proxy configuration. It may contain proxy addresses or
/// sign-ins, so its contents are never printed. Equality only compares snapshots from this OS.
#[derive(Clone, PartialEq, Eq)]
pub struct ProxyFingerprint(Vec<Option<Vec<u8>>>);

impl std::fmt::Debug for ProxyFingerprint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ProxyFingerprint")
    }
}

/// Read the current native proxy configuration without resolving a destination or downloading PAC.
/// A changed value means clients must close old routes before using the next request.
///
/// # Errors
/// A fixed, credential-free reason when the configuration cannot be read safely. The caller must
/// keep existing automatic routes closed until a later read succeeds.
pub fn fingerprint() -> Result<ProxyFingerprint, &'static str> {
    #[cfg(target_os = "linux")]
    return Ok(ProxyFingerprint(Vec::new()));
    #[cfg(windows)]
    return windows::fingerprint();
    #[cfg(target_os = "macos")]
    return macos::fingerprint();
    #[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
    Err("System proxy discovery is unavailable.")
}

/// Query native proxy settings for an allowed URL. `direct://` is an explicit direct result.
///
/// # Errors
/// A fixed error if native lookup fails; callers must not infer DIRECT from a failure.
pub fn resolve(url: &str) -> Result<Vec<String>, &'static str> {
    #[cfg(target_os = "linux")]
    return linux::resolve(url);
    #[cfg(windows)]
    return windows::resolve(url);
    #[cfg(target_os = "macos")]
    return macos::resolve(url);
    #[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
    {
        let _ = url;
        Err("System proxy discovery is unavailable.")
    }
}

#[cfg(test)]
mod fingerprint_tests {
    use super::*;

    #[test]
    fn snapshot_is_stable_and_debug_hides_proxy_values() {
        let first = fingerprint().expect("the system proxy snapshot");
        for _ in 0..32 {
            let next = fingerprint().expect("the unchanged system proxy snapshot");
            assert_eq!(first, next);
        }
        let secret = ProxyFingerprint(vec![Some(b"user:password@proxy.invalid".to_vec())]);
        assert_eq!(format!("{secret:?}"), "ProxyFingerprint");
        assert_ne!(first, secret);
    }

    #[test]
    fn field_boundaries_and_missing_values_change_the_snapshot() {
        let joined = ProxyFingerprint(vec![Some(b"ab".to_vec()), Some(b"c".to_vec())]);
        let separated = ProxyFingerprint(vec![Some(b"a".to_vec()), Some(b"bc".to_vec())]);
        let missing = ProxyFingerprint(vec![None, Some(b"c".to_vec())]);
        let empty = ProxyFingerprint(vec![Some(Vec::new()), Some(b"c".to_vec())]);
        assert_ne!(joined, separated);
        assert_ne!(missing, empty);
    }
}
