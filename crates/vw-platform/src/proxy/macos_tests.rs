//! Injected `CFNetwork` answer dictionaries for macOS route translation. No desktop settings or network access.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use core_foundation_sys::{
    array::{CFArrayAppendValue, CFArrayCreateMutable},
    dictionary::{
        CFDictionaryCreateMutable, CFDictionarySetValue, kCFTypeDictionaryKeyCallBacks,
        kCFTypeDictionaryValueCallBacks,
    },
    number::CFNumberCreate,
};

unsafe extern "C" {
    static kCFNetworkProxiesProxyAutoConfigEnable: CFStringRef;
    static kCFNetworkProxiesProxyAutoConfigJavaScript: CFStringRef;
    static kCFNetworkProxiesProxyAutoConfigURLString: CFStringRef;
}

fn text(value: &str) -> Owned {
    // SAFETY: the UTF-8 bytes remain live during creation; Owned releases the resulting CF string.
    unsafe {
        Owned::new(
            CFStringCreateWithBytes(
                ptr::null(),
                value.as_ptr(),
                CFIndex::try_from(value.len()).expect("short CF fixture string"),
                kCFStringEncodingUTF8,
                0,
            )
            .cast(),
        )
        .expect("CF string fixture")
    }
}

fn entry(kind: Option<CFStringRef>, host: Option<&str>, port: Option<i32>) -> Owned {
    // SAFETY: CF callbacks retain every key and value. Temporary values are released after insertion.
    unsafe {
        let dict = CFDictionaryCreateMutable(
            ptr::null(),
            0,
            &raw const kCFTypeDictionaryKeyCallBacks,
            &raw const kCFTypeDictionaryValueCallBacks,
        );
        let owned = Owned::new(dict.cast()).expect("CF dictionary fixture");
        if let Some(kind) = kind {
            CFDictionarySetValue(dict, kCFProxyTypeKey.cast(), kind.cast());
        }
        if let Some(host) = host {
            let value = text(host);
            CFDictionarySetValue(dict, kCFProxyHostNameKey.cast(), value.0);
        }
        if let Some(port) = port {
            let value = Owned::new(
                CFNumberCreate(ptr::null(), kCFNumberSInt32Type, (&raw const port).cast()).cast(),
            )
            .expect("CF port fixture");
            CFDictionarySetValue(dict, kCFProxyPortNumberKey.cast(), value.0);
        }
        owned
    }
}

fn answer(entries: &[Owned]) -> Owned {
    // SAFETY: CF array callbacks retain each dictionary until the array is released.
    unsafe {
        let array = CFArrayCreateMutable(
            ptr::null(),
            0,
            &raw const core_foundation_sys::array::kCFTypeArrayCallBacks,
        );
        let owned = Owned::new(array.cast()).expect("CF array fixture");
        for entry in entries {
            CFArrayAppendValue(array, entry.0);
        }
        owned
    }
}

fn translated(entries: &[Owned], depth: usize) -> Result<Vec<String>, &'static str> {
    let list = answer(entries);
    routes(list.0.cast(), ptr::null(), depth)
}

#[test]
fn pac_http_socks_and_explicit_direct_keep_order() {
    let entries = [
        entry(
            Some(unsafe { kCFProxyTypeHTTP }),
            Some("proxy.test"),
            Some(8080),
        ),
        entry(
            Some(unsafe { kCFProxyTypeSOCKS }),
            Some("socks.test"),
            Some(1080),
        ),
        entry(Some(unsafe { kCFProxyTypeNone }), None, None),
    ];
    assert_eq!(
        translated(&entries, 1).unwrap(),
        [
            "http://proxy.test:8080/",
            // `url` serializes a non-special scheme with an empty path (no trailing slash); the route parser accepts both.
            "socks5://socks.test:1080",
            "direct://"
        ]
    );
}

#[test]
fn pac_and_manual_https_types_mean_http_connect() {
    let entries = [entry(
        Some(unsafe { kCFProxyTypeHTTPS }),
        Some("proxy.test"),
        Some(8080),
    )];
    assert_eq!(
        translated(&entries, 1).unwrap(),
        ["http://proxy.test:8080/"]
    );
    assert_eq!(
        translated(&entries, 0).unwrap(),
        ["http://proxy.test:8080/"]
    );
}

#[test]
fn native_pac_proxy_for_https_uses_http_connect_route() {
    // SAFETY: every Create/Copy result is retained by Owned and released after the assertions.
    unsafe {
        let script = text("function FindProxyForURL(u,h){return \"PROXY 127.0.0.1:19820\";}");
        let target_text = text("https://api.mistral.ai/");
        let target = Owned::new(
            CFURLCreateWithString(ptr::null(), target_text.0.cast(), ptr::null()).cast(),
        )
        .unwrap();
        let mut error = ptr::null_mut();
        let result = CFNetworkCopyProxiesForAutoConfigurationScript(
            script.0.cast(),
            target.0.cast(),
            &raw mut error,
        );
        assert!(error.is_null(), "native PAC evaluation failed");
        let result = Owned::new(result.cast()).unwrap();
        assert_eq!(CFArrayGetCount(result.0.cast()), 1);
        let entry = CFArrayGetValueAtIndex(result.0.cast(), 0).cast();
        assert!(CFEqual(value(entry, kCFProxyTypeKey), kCFProxyTypeHTTPS.cast()) != 0);
        assert_eq!(
            string(value(entry, kCFProxyHostNameKey)).unwrap(),
            "127.0.0.1"
        );
        assert!(value(entry, kCFProxyUsernameKey).is_null());
        assert!(value(entry, kCFProxyPasswordKey).is_null());
        assert_eq!(
            routes(result.0.cast(), target.0.cast(), 1).unwrap(),
            ["http://127.0.0.1:19820/"]
        );
    }
}

fn system_pac_routes(source_key: CFStringRef, source: &str, target_url: &str) -> Vec<String> {
    // Exercise the outer CFNetwork lookup as well as PAC execution.
    unsafe {
        let settings = CFDictionaryCreateMutable(
            ptr::null(),
            0,
            &raw const kCFTypeDictionaryKeyCallBacks,
            &raw const kCFTypeDictionaryValueCallBacks,
        );
        let settings = Owned::new(settings.cast()).unwrap();
        let enabled: i32 = 1;
        let enabled = Owned::new(
            CFNumberCreate(
                ptr::null(),
                kCFNumberSInt32Type,
                (&raw const enabled).cast(),
            )
            .cast(),
        )
        .unwrap();
        let source = text(source);
        CFDictionarySetValue(
            settings.0.cast_mut().cast(),
            kCFNetworkProxiesProxyAutoConfigEnable.cast(),
            enabled.0,
        );
        CFDictionarySetValue(settings.0.cast_mut().cast(), source_key.cast(), source.0);
        let target_text = text(target_url);
        let target = Owned::new(
            CFURLCreateWithString(ptr::null(), target_text.0.cast(), ptr::null()).cast(),
        )
        .unwrap();
        let initial =
            Owned::new(CFNetworkCopyProxiesForURL(target.0.cast(), settings.0.cast()).cast())
                .unwrap();
        routes(initial.0.cast(), target.0.cast(), 0).unwrap()
    }
}

#[test]
fn system_pac_dead_proxy_does_not_add_implicit_direct() {
    let script = "function FindProxyForURL(url, host) { return host == 'api.mistral.ai' ? 'PROXY 127.0.0.1:19999' : 'DIRECT'; }";
    assert_eq!(
        system_pac_routes(
            unsafe { kCFNetworkProxiesProxyAutoConfigJavaScript },
            script,
            "https://api.mistral.ai/v1/models",
        ),
        ["http://127.0.0.1:19999/"]
    );
    assert_eq!(
        system_pac_routes(
            unsafe { kCFNetworkProxiesProxyAutoConfigJavaScript },
            script,
            "https://example.com/",
        ),
        ["direct://"]
    );
}

#[test]
fn system_pac_explicit_direct_fallback_preserves_order() {
    let script = "function FindProxyForURL(url, host) { return 'PROXY 127.0.0.1:19999; DIRECT'; }";
    assert_eq!(
        system_pac_routes(
            unsafe { kCFNetworkProxiesProxyAutoConfigJavaScript },
            script,
            "https://api.mistral.ai/v1/models",
        ),
        ["http://127.0.0.1:19999/", "direct://"]
    );
}

#[test]
fn system_pac_url_dead_proxy_does_not_add_implicit_direct() {
    use std::io::{Read as _, Write as _};

    let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = server.local_addr().unwrap();
    server.set_nonblocking(true).unwrap();
    let pac_thread = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            match server.accept() {
                Ok((mut client, _)) => {
                    client.set_nonblocking(false).unwrap();
                    client
                        .set_read_timeout(Some(std::time::Duration::from_secs(4)))
                        .unwrap();
                    let mut request = Vec::new();
                    // CFNetwork may close an initial socket before its PAC GET. Answer the
                    // complete request on the next connection without sending on a closed socket.
                    while !request.ends_with(b"\r\n\r\n") && request.len() < 8192 {
                        let mut byte = [0];
                        if client.read(&mut byte).unwrap_or(0) == 0 {
                            break;
                        }
                        request.push(byte[0]);
                    }
                    if !request.ends_with(b"\r\n\r\n") {
                        continue;
                    }
                    let script = "function FindProxyForURL(url, host) { return host == 'api.mistral.ai' ? 'PROXY 127.0.0.1:19999' : 'DIRECT'; }";
                    let reply = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/x-ns-proxy-autoconfig\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{script}",
                        script.len()
                    );
                    client.write_all(reply.as_bytes()).unwrap();
                    return;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "PAC URL was not fetched"
                    );
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(error) => panic!("PAC URL accept failed: {error}"),
            }
        }
    });
    assert_eq!(
        system_pac_routes(
            unsafe { kCFNetworkProxiesProxyAutoConfigURLString },
            &format!("http://{address}/proxy.pac"),
            "https://api.mistral.ai/v1/models",
        ),
        ["http://127.0.0.1:19999/"]
    );
    pac_thread.join().unwrap();
}

#[test]
fn empty_and_incomplete_system_answers_fail_closed() {
    assert_eq!(translated(&[], 0), Err(FAILED));
    assert_eq!(translated(&[entry(None, None, None)], 0), Err(FAILED));
    assert_eq!(
        translated(
            &[entry(Some(unsafe { kCFProxyTypeHTTP }), None, Some(8080))],
            1
        ),
        Err(FAILED)
    );
    assert_eq!(
        translated(
            &[entry(
                Some(unsafe { kCFProxyTypeHTTP }),
                Some("proxy.test"),
                None
            )],
            1
        ),
        Err(FAILED)
    );
}
