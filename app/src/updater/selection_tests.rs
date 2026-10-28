//! Announcement selection regressions, using the updater plugin against a loopback response.
//! Depends on Tauri's mock runtime and the real updater (not on Windows, see below); called by the app's updater tests.
//! An accepted response must retain its minimum and bind it to exactly one selected package.

#[cfg(not(windows))]
use std::io::{Read, Write};
#[cfg(not(windows))]
use std::net::TcpListener;
#[cfg(not(windows))]
use std::time::Duration;

use serde_json::{Value, json};
#[cfg(not(windows))]
use tauri_plugin_updater::{Update, UpdaterExt};

use super::entry_minimum;

// The plugin-backed tests build a mock Tauri app. On Windows that pulls code into the test program which needs the
// common-controls v6 manifest, and Windows then refuses to start the whole test program (STATUS_ENTRYPOINT_NOT_FOUND),
// so `cargo test --workspace` would fail there. The plugin's response handling is the same on every system; the
// selection itself is tested on every system by the pure tests below.
#[cfg(not(windows))]
async fn announced(raw: Value) -> Update {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/announcement", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut request = [0; 4096];
        let count = stream.read(&mut request).unwrap();
        assert!(request[..count].starts_with(b"GET /announcement "));
        let body = raw.to_string();
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
    });
    let mut context = tauri::test::mock_context(tauri::test::noop_assets());
    context.config_mut().plugins.0.insert(
        "updater".into(),
        json!({"pubkey": "", "dangerousInsecureTransportProtocol": true}),
    );
    let app = tauri::test::mock_builder()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .build(context)
        .unwrap();
    let update = app
        .updater_builder()
        .endpoints(vec![endpoint.parse().unwrap()])
        .unwrap()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap()
        .check()
        .await
        .unwrap()
        .unwrap();
    server.join().unwrap();
    update
}

#[cfg(not(windows))]
#[tokio::test]
async fn a_dynamic_unreadable_minimum_is_a_bad_announcement() {
    let update = announced(json!({
        "version": "99.0.0",
        "url": "https://example.invalid/package",
        "signature": "package-signature",
        "minimumGlibc": 7,
        "minimumSystemVersion": 7,
    }))
    .await;
    let error = super::offer_of(&update)
        .expect_err("a present numeric minimum must fail before an update is offered");
    assert!(error.contains("unreadable minimum"), "{error}");
    assert!(error.contains("nothing was downloaded"), "{error}");
}

#[cfg(not(windows))]
#[tokio::test]
async fn duplicate_package_bindings_are_a_bad_announcement() {
    let target = tauri_plugin_updater::target().unwrap();
    let update = announced(json!({
        "version": "99.0.0",
        "platforms": {
            "a-other": {
                "url": "https://example.invalid/package",
                "signature": "package-signature",
                "minimumGlibc": "0.0",
                "minimumSystemVersion": "0.0",
            },
            target: {
                "url": "https://example.invalid/package",
                "signature": "package-signature",
                "minimumGlibc": "99.0",
                "minimumSystemVersion": "99.0",
            },
        },
    }))
    .await;
    let error = super::offer_of(&update)
        .expect_err("duplicate URL/signature bindings must fail before an update is offered");
    assert!(error.contains("ambiguous package entries"), "{error}");
    assert!(error.contains("nothing was downloaded"), "{error}");
}

#[test]
fn dynamic_minimums_reach_validation_including_when_platforms_is_null() {
    let url = "https://example.invalid/package".parse().unwrap();
    for (os, field) in [
        ("Linux", "minimumGlibc"),
        ("macOS", "minimumSystemVersion"),
        ("Windows", "minimumSystemVersion"),
    ] {
        for minimum in [json!(7), json!(null), json!("soon"), json!(true), json!([])] {
            for null_platforms in [false, true] {
                let mut raw = json!({
                    "version": "99.0.0", "url": url, "signature": "sig", field: minimum,
                });
                if null_platforms {
                    raw["platforms"] = Value::Null;
                }
                let release: tauri_plugin_updater::RemoteRelease =
                    serde_json::from_value(raw.clone()).unwrap();
                assert_eq!(release.download_url("chosen").unwrap(), &url);
                let found = entry_minimum(&raw, &url, "sig", "chosen", None, field).unwrap();
                assert_eq!(found, Some(&minimum));
                assert!(vw_host::update_os::check_minimum(os, None, found).is_err());
            }
        }
    }
}

#[cfg(not(windows))]
#[tokio::test]
async fn an_absent_dynamic_minimum_still_offers_the_update() {
    for platforms in [None, Some(Value::Null)] {
        let mut raw = json!({
            "version": "99.0.0",
            "url": "https://example.invalid/package",
            "signature": "package-signature",
        });
        if let Some(platforms) = platforms {
            raw["platforms"] = platforms;
        }
        assert!(super::offer_of(&announced(raw).await).is_ok());
    }
}

#[test]
fn installer_entries_precede_the_plain_target_with_fallback_only_when_absent() {
    let url = "https://example.invalid/package".parse().unwrap();
    for (target, installer, field) in [
        ("linux-x86_64", "appimage", "minimumGlibc"),
        ("linux-x86_64", "deb", "minimumGlibc"),
        ("linux-x86_64", "rpm", "minimumGlibc"),
        ("darwin-aarch64", "app", "minimumSystemVersion"),
        ("windows-x86_64", "msi", "minimumSystemVersion"),
        ("windows-x86_64", "nsis", "minimumSystemVersion"),
    ] {
        let key = format!("{target}-{installer}");
        let mut raw = json!({"platforms": {
            target: {"url": url, "signature": "plain", field: "0.0"},
            &key: {"url": url, "signature": "installer", field: "99.0"},
        }});
        assert_eq!(
            entry_minimum(&raw, &url, "installer", target, Some(installer), field),
            Ok(Some(&json!("99.0")))
        );
        assert!(
            entry_minimum(&raw, &url, "plain", target, Some(installer), field).is_err(),
            "a mismatched installer entry must not fall back to the plain target"
        );
        assert_eq!(
            entry_minimum(&raw, &url, "plain", target, None, field),
            Ok(Some(&json!("0.0")))
        );
        raw["platforms"].as_object_mut().unwrap().remove(&key);
        assert_eq!(
            entry_minimum(&raw, &url, "plain", target, Some(installer), field),
            Ok(Some(&json!("0.0")))
        );
    }
}

#[test]
fn both_package_fields_must_match_the_selected_entry() {
    let url = "https://example.invalid/package".parse().unwrap();
    for entry in [
        json!({"url": url, "signature": "other"}),
        json!({"url": "https://example.invalid/other", "signature": "sig"}),
        json!({"url": url}),
        json!({"signature": "sig"}),
        json!({"url": 7, "signature": "sig"}),
    ] {
        for raw in [entry.clone(), json!({"platforms": {"chosen": entry}})] {
            assert!(entry_minimum(&raw, &url, "sig", "chosen", None, "minimumGlibc").is_err());
        }
    }
    let raw = json!({"platforms": {
        "other": {"url": url, "signature": "sig", "minimumGlibc": "0.0"},
    }});
    assert!(entry_minimum(&raw, &url, "sig", "chosen", None, "minimumGlibc").is_err());
}

#[test]
fn duplicate_bindings_fail_even_if_their_minimums_agree_or_are_absent() {
    let url = "https://example.invalid/package".parse().unwrap();
    for key in ["a-other", "z-other"] {
        for minimum in [
            Some(json!("99.0")),
            Some(json!("0.0")),
            Some(json!(null)),
            None,
        ] {
            let mut other = json!({"url": url, "signature": "sig"});
            if let Some(minimum) = minimum {
                other["minimumGlibc"] = minimum;
            }
            let raw = json!({"platforms": {
                "chosen": {"url": url, "signature": "sig", "minimumGlibc": "99.0"},
                key: other,
            }});
            assert_eq!(
                entry_minimum(&raw, &url, "sig", "chosen", None, "minimumGlibc"),
                Err("ambiguous package entries".into())
            );
        }
    }
}

#[test]
fn static_platforms_take_precedence_over_top_level_fields() {
    let url = "https://example.invalid/package".parse().unwrap();
    let mut raw = json!({
        "version": "99.0.0", "url": url, "signature": "sig", "minimumGlibc": 7,
        "platforms": {"chosen": {"url": url, "signature": "sig", "minimumGlibc": "2.39"}},
    });
    let release: tauri_plugin_updater::RemoteRelease = serde_json::from_value(raw.clone()).unwrap();
    assert!(matches!(
        release.data,
        tauri_plugin_updater::RemoteReleaseInner::Static { .. }
    ));
    assert_eq!(
        entry_minimum(&raw, &url, "sig", "chosen", None, "minimumGlibc"),
        Ok(Some(&json!("2.39")))
    );
    for platforms in [json!({}), json!(7), json!([])] {
        raw["platforms"] = platforms;
        assert!(entry_minimum(&raw, &url, "sig", "chosen", None, "minimumGlibc").is_err());
    }
}
