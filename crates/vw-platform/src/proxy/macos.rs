//! System Configuration through `CFNetwork`, including native PAC execution. Used only by the optional guard.
//! CF objects follow Create/Copy/Get ownership; PAC callbacks run on this worker's private run loop, bounded to 12s.
use core_foundation_sys::{
    array::{CFArrayGetCount, CFArrayGetTypeID, CFArrayGetValueAtIndex, CFArrayRef},
    base::{CFEqual, CFGetTypeID, CFIndex, CFRelease, CFRetain, CFTypeRef},
    dictionary::{CFDictionaryGetTypeID, CFDictionaryGetValue, CFDictionaryRef},
    error::CFErrorRef,
    number::{CFNumberGetTypeID, CFNumberGetValue, kCFNumberSInt32Type},
    runloop::{
        CFRunLoopAddSource, CFRunLoopGetCurrent, CFRunLoopRemoveSource, CFRunLoopRunInMode,
        CFRunLoopSourceInvalidate, CFRunLoopSourceRef, kCFRunLoopDefaultMode,
    },
    string::{
        CFStringCreateWithBytes, CFStringGetCString, CFStringGetLength,
        CFStringGetMaximumSizeForEncoding, CFStringGetTypeID, CFStringRef, kCFStringEncodingUTF8,
    },
    url::{CFURLCreateWithString, CFURLGetTypeID, CFURLRef},
};
use std::ffi::{c_char, c_void};
use std::ptr;
const FAILED: &str = "macOS proxy resolution failed.";

#[link(name = "CFNetwork", kind = "framework")]
unsafe extern "C" {
    fn CFNetworkCopySystemProxySettings() -> CFDictionaryRef;
    fn CFNetworkCopyProxiesForURL(url: CFURLRef, settings: CFDictionaryRef) -> CFArrayRef;
    fn CFNetworkCopyProxiesForAutoConfigurationScript(
        script: CFStringRef,
        url: CFURLRef,
        error: *mut CFErrorRef,
    ) -> CFArrayRef;
    fn CFNetworkExecuteProxyAutoConfigurationURL(
        pac: CFURLRef,
        target: CFURLRef,
        callback: extern "C" fn(*mut c_void, CFArrayRef, CFErrorRef),
        context: *mut Context,
    ) -> CFRunLoopSourceRef;
    static kCFProxyTypeKey: CFStringRef;
    static kCFProxyTypeNone: CFStringRef;
    static kCFProxyTypeHTTP: CFStringRef;
    static kCFProxyTypeHTTPS: CFStringRef;
    static kCFProxyTypeSOCKS: CFStringRef;
    static kCFProxyTypeAutoConfigurationURL: CFStringRef;
    static kCFProxyTypeAutoConfigurationJavaScript: CFStringRef;
    static kCFProxyHostNameKey: CFStringRef;
    static kCFProxyPortNumberKey: CFStringRef;
    static kCFProxyUsernameKey: CFStringRef;
    static kCFProxyPasswordKey: CFStringRef;
    static kCFProxyAutoConfigurationURLKey: CFStringRef;
    static kCFProxyAutoConfigurationJavaScriptKey: CFStringRef;
}
#[repr(C)]
struct Context {
    version: CFIndex,
    info: *mut c_void,
    retain: Option<extern "C" fn(*const c_void) -> *const c_void>,
    release: Option<extern "C" fn(*const c_void)>,
    description: Option<extern "C" fn(*const c_void) -> CFStringRef>,
}
struct Owned(CFTypeRef);
impl Owned {
    fn new(value: CFTypeRef) -> Result<Self, &'static str> {
        if value.is_null() {
            Err(FAILED)
        } else {
            Ok(Self(value))
        }
    }
}
impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: Owned holds exactly one Create/Copy/Retain reference, never null.
        unsafe {
            CFRelease(self.0);
        }
    }
}

pub(super) fn resolve(url: &str) -> Result<Vec<String>, &'static str> {
    // SAFETY: all inputs and Create/Copy outputs are owned for the entire call chain; typed values are checked.
    unsafe {
        let bytes = url.as_bytes();
        let string = Owned::new(
            CFStringCreateWithBytes(
                ptr::null(),
                bytes.as_ptr(),
                CFIndex::try_from(bytes.len()).map_err(|_| FAILED)?,
                kCFStringEncodingUTF8,
                0,
            )
            .cast(),
        )?;
        let target =
            Owned::new(CFURLCreateWithString(ptr::null(), string.0.cast(), ptr::null()).cast())?;
        let settings = Owned::new(CFNetworkCopySystemProxySettings().cast())?;
        let proxies =
            Owned::new(CFNetworkCopyProxiesForURL(target.0.cast(), settings.0.cast()).cast())?;
        routes(proxies.0.cast(), target.0.cast(), 0)
    }
}

fn value(dict: CFDictionaryRef, key: CFStringRef) -> CFTypeRef {
    // SAFETY: caller holds a validated dictionary; keys are CFNetwork string constants.
    unsafe { CFDictionaryGetValue(dict, key.cast()) }
}
fn string(value: CFTypeRef) -> Result<String, &'static str> {
    // SAFETY: type checked before CFString calls; sized buffer includes terminator and lives through the copy.
    unsafe {
        if value.is_null() || CFGetTypeID(value) != CFStringGetTypeID() {
            return Err(FAILED);
        }
        let len = CFStringGetMaximumSizeForEncoding(
            CFStringGetLength(value.cast()),
            kCFStringEncodingUTF8,
        );
        if !(0..=65536).contains(&len) {
            return Err(FAILED);
        }
        let mut bytes = vec![0u8; usize::try_from(len).map_err(|_| FAILED)? + 1];
        if CFStringGetCString(
            value.cast(),
            bytes.as_mut_ptr().cast::<c_char>(),
            len + 1,
            kCFStringEncodingUTF8,
        ) == 0
        {
            return Err(FAILED);
        }
        let end = bytes.iter().position(|b| *b == 0).ok_or(FAILED)?;
        String::from_utf8(bytes[..end].to_vec()).map_err(|_| FAILED)
    }
}
fn optional_string(
    dict: CFDictionaryRef,
    key: CFStringRef,
) -> Result<Option<String>, &'static str> {
    let v = value(dict, key);
    if v.is_null() {
        Ok(None)
    } else {
        string(v).map(Some)
    }
}

fn routes(array: CFArrayRef, target: CFURLRef, depth: usize) -> Result<Vec<String>, &'static str> {
    // SAFETY: array is owned by caller; every item and number is type checked before use; Get values stay borrowed.
    unsafe {
        if depth > 2 || CFGetTypeID(array.cast()) != CFArrayGetTypeID() {
            return Err(FAILED);
        }
        let count = CFArrayGetCount(array);
        if !(1..=32).contains(&count) {
            return Err(FAILED);
        }
        let mut out = Vec::new();
        for i in 0..count {
            let raw = CFArrayGetValueAtIndex(array, i);
            if raw.is_null() || CFGetTypeID(raw) != CFDictionaryGetTypeID() {
                return Err(FAILED);
            }
            let dict = raw.cast();
            let kind = value(dict, kCFProxyTypeKey);
            if kind.is_null() {
                return Err(FAILED);
            }
            let is = |ty: CFStringRef| CFEqual(kind, ty.cast()) != 0;
            if is(kCFProxyTypeNone) {
                out.push("direct://".to_owned());
            } else if is(kCFProxyTypeAutoConfigurationURL)
                || is(kCFProxyTypeAutoConfigurationJavaScript)
            {
                let resolved = if is(kCFProxyTypeAutoConfigurationURL) {
                    pac_url(value(dict, kCFProxyAutoConfigurationURLKey), target)?
                } else {
                    let script = value(dict, kCFProxyAutoConfigurationJavaScriptKey);
                    if script.is_null() || CFGetTypeID(script) != CFStringGetTypeID() {
                        return Err(FAILED);
                    }
                    let mut error = ptr::null_mut();
                    let result = CFNetworkCopyProxiesForAutoConfigurationScript(
                        script.cast(),
                        target,
                        &raw mut error,
                    );
                    if !error.is_null() {
                        CFRelease(error.cast());
                        if !result.is_null() {
                            CFRelease(result.cast());
                        }
                        return Err(FAILED);
                    }
                    Owned::new(result.cast())?
                };
                out.extend(routes(resolved.0.cast(), target, depth + 1)?);
            } else {
                // CFNetwork's HTTPS setting means an HTTP proxy for HTTPS destinations, not TLS to
                // the proxy (same mapping as Chromium's proxy_chain_util_apple). PAC output cannot
                // disambiguate a TLS-proxy extension: refuse that case instead of guessing cleartext.
                if depth > 0 && is(kCFProxyTypeHTTPS) {
                    return Err(FAILED);
                }
                let scheme = if is(kCFProxyTypeHTTP) || is(kCFProxyTypeHTTPS) {
                    "http"
                } else if is(kCFProxyTypeSOCKS) {
                    "socks5"
                } else {
                    return Err(FAILED);
                };
                let host = string(value(dict, kCFProxyHostNameKey))?;
                let number = value(dict, kCFProxyPortNumberKey);
                if number.is_null() || CFGetTypeID(number) != CFNumberGetTypeID() {
                    return Err(FAILED);
                }
                let mut port: i32 = 0;
                if !CFNumberGetValue(number.cast(), kCFNumberSInt32Type, (&raw mut port).cast()) {
                    return Err(FAILED);
                }
                out.push(super::manual::endpoint(
                    scheme,
                    &host,
                    u16::try_from(port).map_err(|_| FAILED)?,
                    optional_string(dict, kCFProxyUsernameKey)?.as_deref(),
                    optional_string(dict, kCFProxyPasswordKey)?.as_deref(),
                )?);
            }
            if out.len() > 32 {
                return Err(FAILED);
            }
        }
        Ok(out)
    }
}

struct Answer {
    value: Option<Owned>,
    done: bool,
}
extern "C" fn completed(context: *mut c_void, proxies: CFArrayRef, error: CFErrorRef) {
    // SAFETY: context points to the worker's Answer until its source is invalidated. This callback is dispatched
    // only on that worker's run loop; retain the borrowed result before CFNetwork releases it.
    unsafe {
        let answer = &mut *context.cast::<Answer>();
        if error.is_null() && !proxies.is_null() {
            answer.value = Some(Owned(CFRetain(proxies.cast())));
        }
        answer.done = true;
    }
}
fn pac_url(pac: CFTypeRef, target: CFURLRef) -> Result<Owned, &'static str> {
    // SAFETY: PAC URL is type checked. The source is removed/invalidated before stack context drops, so no late
    // callback can access it. Native fetching/evaluation owns PAC semantics and cache, not the guard.
    unsafe {
        if pac.is_null() || CFGetTypeID(pac) != CFURLGetTypeID() {
            return Err(FAILED);
        }
        let mut answer = Answer {
            value: None,
            done: false,
        };
        let mut context = Context {
            version: 0,
            info: (&raw mut answer).cast(),
            retain: None,
            release: None,
            description: None,
        };
        let source = Owned::new(
            CFNetworkExecuteProxyAutoConfigurationURL(
                pac.cast(),
                target,
                completed,
                &raw mut context,
            )
            .cast(),
        )?;
        let run_loop = CFRunLoopGetCurrent();
        CFRunLoopAddSource(run_loop, source.0.cast_mut().cast(), kCFRunLoopDefaultMode);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(12);
        while !answer.done && std::time::Instant::now() < deadline {
            CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.1, 1);
        }
        CFRunLoopRemoveSource(run_loop, source.0.cast_mut().cast(), kCFRunLoopDefaultMode);
        CFRunLoopSourceInvalidate(source.0.cast_mut().cast());
        answer.value.ok_or(FAILED)
    }
}
