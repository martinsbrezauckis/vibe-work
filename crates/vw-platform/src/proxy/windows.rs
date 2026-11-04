//! `WinHTTP`/IE proxy settings and native out-of-process PAC/WPAD. Called on a bounded worker.
//! Async callbacks use integer registry keys (never borrowed pointers), so timeouts cannot leave dangling contexts.
use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock, mpsc};
use windows::Win32::Foundation::{ERROR_IO_PENDING, GlobalFree, HGLOBAL};
use windows::Win32::Networking::WinHttp::{
    WINHTTP_ACCESS_TYPE_NO_PROXY, WINHTTP_ASYNC_RESULT, WINHTTP_AUTO_DETECT_TYPE_DHCP,
    WINHTTP_AUTO_DETECT_TYPE_DNS_A, WINHTTP_AUTOPROXY_AUTO_DETECT, WINHTTP_AUTOPROXY_CONFIG_URL,
    WINHTTP_AUTOPROXY_OPTIONS, WINHTTP_CALLBACK_FLAG_GETPROXYFORURL_COMPLETE,
    WINHTTP_CALLBACK_FLAG_REQUEST_ERROR, WINHTTP_CALLBACK_STATUS_GETPROXYFORURL_COMPLETE,
    WINHTTP_CALLBACK_STATUS_REQUEST_ERROR, WINHTTP_CURRENT_USER_IE_PROXY_CONFIG,
    WINHTTP_FLAG_ASYNC, WINHTTP_INTERNET_SCHEME_HTTP, WINHTTP_INTERNET_SCHEME_HTTPS,
    WINHTTP_INTERNET_SCHEME_SOCKS, WINHTTP_PROXY_INFO, WINHTTP_PROXY_RESULT, WinHttpCloseHandle,
    WinHttpCreateProxyResolver, WinHttpFreeProxyResult, WinHttpGetDefaultProxyConfiguration,
    WinHttpGetIEProxyConfigForCurrentUser, WinHttpGetProxyForUrlEx, WinHttpGetProxyResult,
    WinHttpOpen, WinHttpSetStatusCallback, WinHttpSetTimeouts,
};
use windows::core::{PCWSTR, PWSTR, w};

use super::windows_policy::{FAILED, Failure, Mode, Policy};

static POLICY: OnceLock<Policy> = OnceLock::new();
type Waiters = Mutex<HashMap<usize, mpsc::Sender<Result<(), Failure>>>>;
static WAITERS: OnceLock<Waiters> = OnceLock::new();
static NEXT: AtomicUsize = AtomicUsize::new(1);
struct Handle(*mut c_void);
impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: owned WinHTTP handle; no borrowed context is attached to it.
        unsafe {
            let _ = WinHttpCloseHandle(self.0);
        }
    }
}
struct Ie(WINHTTP_CURRENT_USER_IE_PROXY_CONFIG);
impl Drop for Ie {
    fn drop(&mut self) {
        release(self.0.lpszAutoConfigUrl);
        release(self.0.lpszProxy);
        release(self.0.lpszProxyBypass);
    }
}
struct Info(WINHTTP_PROXY_INFO);
impl Drop for Info {
    fn drop(&mut self) {
        release(self.0.lpszProxy);
        release(self.0.lpszProxyBypass);
    }
}
fn release(value: PWSTR) {
    if !value.is_null() {
        // SAFETY: WinHTTP allocates these strings with GlobalAlloc and transfers ownership to the caller.
        unsafe {
            let _ = GlobalFree(Some(HGLOBAL(value.0.cast())));
        }
    }
}
fn string(value: PWSTR) -> Result<String, &'static str> {
    if value.is_null() {
        return Ok(String::new());
    }
    // SAFETY: WinHTTP-owned NUL-terminated UTF-16, copied before the owning config/result is freed.
    unsafe { value.to_string().map_err(|_| FAILED) }
}

pub(super) fn resolve(url: &str) -> Result<Vec<String>, &'static str> {
    // SAFETY: all output structs are initialized, allocations are owned by RAII wrappers.
    unsafe {
        let mut ie = Ie(WINHTTP_CURRENT_USER_IE_PROXY_CONFIG::default());
        let got_ie = WinHttpGetIEProxyConfigForCurrentUser(&raw mut ie.0).is_ok();
        let mode = Mode::from_settings(
            got_ie && ie.0.fAutoDetect.as_bool(),
            got_ie && !ie.0.lpszAutoConfigUrl.is_null(),
        );
        POLICY.get_or_init(Policy::default).resolve(
            mode,
            || automatic(url, &ie.0),
            || manual_or_default(url, got_ie.then_some(&ie.0)),
        )
    }
}

fn manual_or_default(
    url: &str,
    ie: Option<&WINHTTP_CURRENT_USER_IE_PROXY_CONFIG>,
) -> Result<Vec<String>, &'static str> {
    // SAFETY: IE settings are owned by the caller and default settings by the local RAII wrapper.
    unsafe {
        if let Some(ie) = ie.filter(|ie| !ie.lpszProxy.is_null()) {
            return super::manual::routes(
                url,
                &string(ie.lpszProxy)?,
                &string(ie.lpszProxyBypass)?,
            );
        }
        let mut info = Info(WINHTTP_PROXY_INFO::default());
        WinHttpGetDefaultProxyConfiguration(&raw mut info.0).map_err(|_| FAILED)?;
        if info.0.dwAccessType == WINHTTP_ACCESS_TYPE_NO_PROXY {
            return Ok(vec!["direct://".to_owned()]);
        }
        super::manual::routes(
            url,
            &string(info.0.lpszProxy)?,
            &string(info.0.lpszProxyBypass)?,
        )
    }
}

unsafe extern "system" fn callback(
    _: *mut c_void,
    context: usize,
    status: u32,
    information: *mut c_void,
    information_len: u32,
) {
    if !matches!(
        status,
        WINHTTP_CALLBACK_STATUS_GETPROXYFORURL_COMPLETE | WINHTTP_CALLBACK_STATUS_REQUEST_ERROR
    ) {
        return;
    }
    if let Some(waiters) = WAITERS.get()
        && let Some(sender) = waiters
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&context)
    {
        let result = if status == WINHTTP_CALLBACK_STATUS_GETPROXYFORURL_COMPLETE {
            Ok(())
        } else if !information.is_null()
            && information_len as usize >= size_of::<WINHTTP_ASYNC_RESULT>()
        {
            // SAFETY: REQUEST_ERROR carries WINHTTP_ASYNC_RESULT, valid only during this callback.
            // Copy dwError, never retain the native pointer or credential-bearing diagnostic strings.
            let error =
                unsafe { information.cast::<WINHTTP_ASYNC_RESULT>().read_unaligned() }.dwError;
            Err(Failure::WinHttp(error))
        } else {
            Err(Failure::Internal)
        };
        let _ = sender.send(result);
    }
}

fn automatic(url: &str, ie: &WINHTTP_CURRENT_USER_IE_PROXY_CONFIG) -> Result<Vec<String>, Failure> {
    // SAFETY: inputs live through the lookup; WinHTTP's async completion carries only a numeric registry ID.
    // WinHttpGetProxyForUrlEx is the out-of-process version of GetProxyForUrl and preserves ordered DIRECT entries.
    unsafe {
        let session = Handle(WinHttpOpen(
            w!("Vibe Work"),
            WINHTTP_ACCESS_TYPE_NO_PROXY,
            PCWSTR::null(),
            PCWSTR::null(),
            WINHTTP_FLAG_ASYNC,
        ));
        if session.0.is_null() {
            return Err(Failure::Internal);
        }
        WinHttpSetTimeouts(session.0, 5000, 5000, 5000, 5000).map_err(|_| Failure::Internal)?;
        let previous = WinHttpSetStatusCallback(
            session.0,
            Some(callback),
            WINHTTP_CALLBACK_FLAG_REQUEST_ERROR | WINHTTP_CALLBACK_FLAG_GETPROXYFORURL_COMPLETE,
            0,
        );
        if previous.is_some_and(|f| f as usize == usize::MAX) {
            return Err(Failure::Internal);
        }
        let mut resolver = Handle(std::ptr::null_mut());
        if WinHttpCreateProxyResolver(session.0, &raw mut resolver.0) != 0 {
            return Err(Failure::Internal);
        }
        let options = WINHTTP_AUTOPROXY_OPTIONS {
            dwFlags: if ie.lpszAutoConfigUrl.is_null() {
                WINHTTP_AUTOPROXY_AUTO_DETECT
            } else {
                WINHTTP_AUTOPROXY_CONFIG_URL
            },
            dwAutoDetectFlags: if ie.lpszAutoConfigUrl.is_null() {
                WINHTTP_AUTO_DETECT_TYPE_DHCP | WINHTTP_AUTO_DETECT_TYPE_DNS_A
            } else {
                0
            },
            lpszAutoConfigUrl: PCWSTR(ie.lpszAutoConfigUrl.0),
            // Never send logged-in Windows credentials to a discovered PAC server automatically.
            fAutoLogonIfChallenged: false.into(),
            ..Default::default()
        };
        let wide: Vec<u16> = url.encode_utf16().chain(Some(0)).collect();
        let id = next_id().map_err(|_| Failure::Internal)?;
        let (sender, receiver) = mpsc::channel();
        let waiters = WAITERS.get_or_init(Mutex::default);
        waiters
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(id, sender);
        let pending = WinHttpGetProxyForUrlEx(
            resolver.0,
            PCWSTR(wide.as_ptr()),
            &raw const options,
            Some(id),
        );
        let completion = if pending == ERROR_IO_PENDING.0 {
            receiver
                .recv_timeout(std::time::Duration::from_secs(12))
                .unwrap_or_else(|error| {
                    Err(match error {
                        mpsc::RecvTimeoutError::Timeout => Failure::Timeout,
                        mpsc::RecvTimeoutError::Disconnected => Failure::Internal,
                    })
                })
        } else {
            Err(Failure::WinHttp(pending))
        };
        waiters
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&id);
        completion?;
        let mut result = WINHTTP_PROXY_RESULT::default();
        if WinHttpGetProxyResult(resolver.0, &raw mut result) != 0 {
            return Err(Failure::Internal);
        }
        let out = entries(&result);
        WinHttpFreeProxyResult(&raw mut result);
        out.map_err(|_| Failure::Internal)
    }
}

fn entries(result: &WINHTTP_PROXY_RESULT) -> Result<Vec<String>, &'static str> {
    if result.cEntries == 0 || result.cEntries > 32 || result.pEntries.is_null() {
        return Err(FAILED);
    }
    // SAFETY: WinHttpGetProxyResult allocated exactly cEntries items; caller retains it until this copy returns.
    let entries = unsafe { std::slice::from_raw_parts(result.pEntries, result.cEntries as usize) };
    entries
        .iter()
        .map(|entry| {
            if !entry.fProxy.as_bool() || entry.fBypass.as_bool() {
                return Ok("direct://".to_owned());
            }
            let scheme = match entry.ProxyScheme {
                WINHTTP_INTERNET_SCHEME_HTTP => "http",
                WINHTTP_INTERNET_SCHEME_HTTPS => "https",
                WINHTTP_INTERNET_SCHEME_SOCKS => "socks4a",
                _ => return Err(FAILED),
            };
            let host = string(entry.pwszProxy)?;
            super::manual::endpoint(scheme, &host, entry.ProxyPort, None, None)
        })
        .collect()
}

fn next_id() -> Result<usize, &'static str> {
    let mut current = NEXT.load(Ordering::Relaxed);
    loop {
        let next = current.checked_add(1).ok_or(FAILED)?;
        match NEXT.compare_exchange_weak(current, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return Ok(current),
            Err(actual) => current = actual,
        }
    }
}
