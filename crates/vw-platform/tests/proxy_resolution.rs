//! Native resolver smoke in an isolated subprocess. Never edits desktop settings or contacts the Internet.
//! The injected environment proxy is a URL only; PAC fixtures are served on loopback by this test.
#![cfg(all(feature = "proxy-resolver", target_os = "linux"))]
#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::io::{Read, Write};

fn child(mode: &str) {
    let result = vw_platform::proxy::resolve("https://allowed.invalid/path");
    if matches!(mode, "pac-error" | "pac-invalid") {
        assert!(
            result.is_err(),
            "a broken explicit PAC must not become DIRECT"
        );
        return;
    }
    let mut result = result.unwrap();
    // libproxy 0.5.4 (Ubuntu 24.04, the CI runner) drops a PAC answer's trailing DIRECT; newer releases keep it. The
    // shorter list only loses the direct fallback (fails closed), so both are accepted.
    if mode == "pac" && result.len() == 2 {
        result.push("direct://".to_owned());
    }
    let expected = match mode {
        "proxy" => vec!["socks5://user:secret@127.0.0.1:1080"],
        "pac" => vec!["http://127.0.0.1:9", "socks://127.0.0.1:10", "direct://"],
        _ => vec!["direct://"],
    };
    // No Debug formatting of native results: they can carry credentials.
    assert!(
        result.len() == expected.len()
            && result
                .iter()
                .zip(expected)
                .all(|(a, b)| a.trim_end_matches('/') == b.trim_end_matches('/'))
    );
}

#[test]
fn linux_native_environment_and_direct() {
    if let Ok(mode) = std::env::var("VW_NATIVE_PROXY_CHILD") {
        child(&mode);
        return;
    }
    for mode in ["proxy", "direct", "pac", "pac-error", "pac-invalid"] {
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command.args([
            "--exact",
            "linux_native_environment_and_direct",
            "--nocapture",
        ]);
        for key in [
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "NO_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
            "no_proxy",
        ] {
            command.env_remove(key);
        }
        command
            .env("VW_NATIVE_PROXY_CHILD", mode)
            .env("GSETTINGS_BACKEND", "memory")
            .env("PX_FORCE_CONFIG", "config-env")
            .env("PX_DEBUG", "1");
        let mut server = None;
        if mode == "proxy" {
            command.env("https_proxy", "socks5://user:secret@127.0.0.1:1080");
        }
        if mode.starts_with("pac") {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            command.env(
                "https_proxy",
                format!(
                    "pac+http://user:secret@{}/proxy.pac",
                    listener.local_addr().unwrap()
                ),
            );
            if mode == "pac-error" {
                drop(listener);
            } else {
                server = Some(std::thread::spawn(move || {
                    let (mut socket, _) = listener.accept().unwrap();
                    socket
                        .set_read_timeout(Some(std::time::Duration::from_secs(3)))
                        .unwrap();
                    let mut request = [0; 4096];
                    assert!(socket.read(&mut request).unwrap() > 0);
                    let pac = if mode == "pac-invalid" {
                        "function FindProxyForURL( { invalid"
                    } else {
                        "function FindProxyForURL(url, host) { return 'PROXY 127.0.0.1:9; SOCKS 127.0.0.1:10; DIRECT'; }"
                    };
                    write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/x-ns-proxy-autoconfig\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{pac}", pac.len()).unwrap();
                }));
            }
        }
        let output = command.output().unwrap();
        if let Some(server) = server {
            server.join().unwrap();
        }
        assert!(
            !String::from_utf8_lossy(&output.stdout).contains("secret")
                && !String::from_utf8_lossy(&output.stderr).contains("secret"),
            "native resolver logged credentials"
        );
        assert!(
            output.status.success(),
            "native child {mode} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
