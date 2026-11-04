//! OS-owned proxy/PAC resolution for the optional guard. Depends on `WinHTTP`, `CFNetwork` or libproxy.
//! Called on bounded worker threads after destination policy approval. Returns URL lists in OS order;
//! errors never include settings or credentials. No JavaScript evaluator or WPAD discovery is implemented here.
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
mod linux_log;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(windows, target_os = "macos", test))]
mod manual;
#[cfg(windows)]
mod windows;
#[cfg(any(windows, test))]
mod windows_policy;

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
