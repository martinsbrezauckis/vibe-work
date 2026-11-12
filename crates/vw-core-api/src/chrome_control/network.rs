//! Chrome URL arguments use normal browser permissions. Legacy network fields are sent only for Store compatibility.
//! The helper and extension retain site grants, access modes, ownership, protected pages and Stop.
use crate::core::Shared;
use serde_json::{Value, json};
use std::sync::Arc;
use vw_preview::{PortOwner, UrlPolicy};

/// Actions whose `url` argument opens or fetches that address.
const URL_ACTIONS: [&str; 4] = [
    "tabs.create",
    "page.navigate",
    "files.download",
    "auth.basic",
];

/// The rule sent with one call.
pub(super) fn policy(_local_network: bool, loopback_ports: &[u16]) -> Value {
    json!({"localNetwork": true, "loopbackPorts": loopback_ports})
}

/// Checks the call's `url` argument, if its action has one.
///
/// # Errors
/// The refusal for unsupported schemes or credentials in a URL.
pub(super) fn check_url(url_policy: &UrlPolicy, args: &Value) -> Result<(), String> {
    let action = args["action"].as_str().unwrap_or_default();
    let Some(text) = args["args"]["url"]
        .as_str()
        .filter(|_| URL_ACTIONS.contains(&action))
    else {
        return Ok(());
    };
    let refuse = |why: String| format!("Not run: {why}.");
    url_policy
        .check(text, |_| PortOwner::Nobody, true)
        .map_err(|e| refuse(e.to_string()))?;
    Ok(())
}

/// Adds legacy compatibility fields and checks URL schemes and credentials. Browser DNS is not an admission gate.
///
/// # Errors
/// The refusal text.
pub(super) async fn prepare(
    shared: &Arc<Shared>,
    session: &str,
    args: &mut Value,
) -> Result<(), String> {
    let browser = super::agent_browser::selected(shared, session, "chrome_execute", args);
    if browser == super::agent_browser::Browser::Firefox {
        super::local_documents::prepare(shared, session, args).await?;
    }
    if browser == super::agent_browser::Browser::Safari {
        if URL_ACTIONS.contains(&args["action"].as_str().unwrap_or_default()) {
            let parsed = args["args"]["url"]
                .as_str()
                .and_then(|text| url::Url::parse(text).ok())
                .filter(|url| {
                    matches!(url.scheme(), "http" | "https")
                        && url.username().is_empty()
                        && url.password().is_none()
                });
            if parsed.is_none() {
                return Err("Not run: Safari needs an HTTP or HTTPS URL without credentials. Supply a web address.".into());
            }
        }
        return Ok(());
    }
    let local_network = shared.chrome_control.local_network();
    let (scan, id) = (shared.clone(), session.to_owned());
    let ports =
        tokio::task::spawn_blocking(move || crate::preview_ports::session_ports(&scan, &id))
            .await
            .map_err(|_| "Not run: could not read this chat's servers.".to_owned())?
            .map_err(|_| "Not run: could not read this chat's servers.".to_owned())?;
    let default_policy = UrlPolicy::default();
    let url_policy = shared
        .preview
        .as_ref()
        .map_or(&default_policy, |hub| hub.service.policy());
    let mut destinations = vec![args.clone()];
    if args["action"] == "network.intercept" {
        for rule in args["args"]["rules"].as_array().into_iter().flatten() {
            let direct = rule["url"].as_str();
            let headers = rule["headers"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|header| {
                    header["name"]
                        .as_str()
                        .is_some_and(|name| name.eq_ignore_ascii_case("location"))
                })
                .filter_map(|header| header["value"].as_str());
            for destination in direct.into_iter().chain(headers) {
                // Relative redirects are judged against the paused request URL in Chrome. Absolute and
                // scheme-relative destinations are already knowable here, so refuse them before dispatch.
                let url = if destination.starts_with("//") {
                    format!("https:{destination}")
                } else {
                    destination.to_owned()
                };
                if url::Url::parse(&url).is_ok() {
                    destinations.push(json!({"action":"page.navigate","args":{"url":url}}));
                }
            }
        }
    }
    for destination in destinations {
        if destination["action"] == "page.navigate"
            && destination["args"]["url"]
                .as_str()
                .is_some_and(|u| u.starts_with("file:"))
        {
            let text = destination["args"]["url"].as_str().unwrap_or_default();
            crate::browser_local::checked(shared, session, text).await?;
            let result = super::page_plan::dispatch(
                shared,
                session,
                &destination,
                None,
                json!([{"id":"state","op":"api","method":"permissions.status"}]),
            )
            .await?;
            if result["status"] != "completed"
                || result.pointer("/data/results/state/fileAccess") != Some(&json!(true))
            {
                return Err("Local file access is off or could not be confirmed. Ask the user to enable Allow access to file URLs in the extension's browser details, then retry page.navigate with this tabId.".into());
            }
            continue;
        }
        check_url(url_policy, &destination)?;
    }
    args["_netPolicy"] = policy(local_network, &ports.allowed(true));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(action: &str, url: &str) -> Value {
        json!({"sessionId":"s","action":action,"args":{"url":url}})
    }
    fn decide(action: &str, url: &str) -> Result<(), String> {
        check_url(&UrlPolicy::default(), &call(action, url))
    }

    #[test]
    fn local_addresses_use_ordinary_browser_admission() {
        let private = format!("http://{}/", std::net::Ipv4Addr::new(192, 168, 1, 2));
        for action in URL_ACTIONS {
            for url in [
                "http://localhost:9999/",
                "https://127.0.0.2:8443/",
                "http://10.0.0.8/",
                "http://172.16.0.1/",
                private.as_str(),
                "http://169.254.1.1/",
                "http://[fe80::1]/",
                "http://[fd00::1]/",
                "http://nas.local/",
                "http://router/",
                "http://nas.example/",
            ] {
                assert!(decide(action, url).is_ok(), "{action} {url}");
            }
        }
    }

    #[test]
    fn unsupported_schemes_and_credentials_still_refuse() {
        for action in URL_ACTIONS {
            for url in [
                "file:///etc/passwd",
                "javascript:alert(1)",
                "chrome://settings",
                "http://user:password@localhost/",
            ] {
                assert!(decide(action, url).is_err(), "{action} {url}");
            }
        }
    }

    #[tokio::test]
    async fn local_addresses_reach_the_helper_without_a_developer_switch() {
        let core = crate::offline_core::start("chrome-network-gateway").await;
        let shared = &core.shared;
        let _ = shared
            .store
            .load(
                json!({"session":{"id":"s"},"history":[],"turns":[]}),
                vw_session::LoadKind::New,
            )
            .unwrap();
        shared.access.observe_request(
            "session/agent/update",
            "s",
            &json!({"agentName":"accept-edits"}),
        );
        shared
            .chrome_control
            .tasks
            .lock()
            .unwrap()
            .insert("t".into(), "s".into());
        for url in [
            "http://172.16.1.2/",
            "http://169.254.1.1/",
            "http://localhost:9999/",
        ] {
            let passed = super::super::gateway::call(
                shared,
                "s",
                "chrome_execute",
                json!({"sessionId":"t","action":"tabs.create","args":{"url":url}}),
            )
            .await
            .unwrap();
            assert!(passed.to_string().contains("not running"), "{passed}");
        }
        core.shutdown().await;
    }

    #[test]
    fn legacy_transport_is_permissive_and_non_url_actions_stay_unchanged() {
        let read = json!({"sessionId":"s","action":"page.read","args":{"url":"http://10.1.1.1/"}});
        assert!(check_url(&UrlPolicy::default(), &read).is_ok());
        assert_eq!(
            policy(false, &[5173]),
            json!({"localNetwork":true,"loopbackPorts":[5173]})
        );
    }
}

#[cfg(test)]
mod switch_delivery_tests {
    use serde_json::{Value, json};
    use std::sync::{Arc, Mutex};

    /// The person turns Local network pages off while the helper does not take the change (a busy owner pipe
    /// answers BUSY without sending it). The change must stay pending and be sent again until the helper acknowledges
    /// that value. PREDICT (before the change): one delivery, never retried.
    #[tokio::test]
    async fn a_switch_change_the_helper_did_not_take_is_sent_again_until_acknowledged() {
        let core = crate::offline_core::start_with("chrome-switch-retry", |config, dir| {
            config.chrome_control.package = Some(dir.join("chrome-package"));
        })
        .await;
        let shared = &core.shared;
        let deliveries: Arc<Mutex<Vec<Value>>> = Arc::default();
        let log = deliveries.clone();
        let _seen = super::super::test_support::answering(shared, move |_, args| {
            let Some(change) = args.get("_netPolicyChange") else {
                return json!({"isError":false,"content":[],"structuredContent":{}});
            };
            let mut log = log.lock().unwrap();
            log.push(change.clone());
            if log.len() == 2 {
                // The first "off": not taken (the core's own BUSY has this shape).
                json!({"isError":false,"content":[{"type":"text","text":"Not run: Chrome is busy."}],
                    "structuredContent":{"error":{"code":"BUSY","message":"Not run: Chrome is busy."}}})
            } else {
                json!({"isError":false,"content":[],"structuredContent":{"revision":log.len(),"pendingProfiles":[]}})
            }
        });
        for on in [true, false] {
            shared.chrome_control.set_local_network(on);
            super::super::send_local_network(shared).await;
        }
        // Not taken yet: the view says so.
        assert!(!super::super::commands::view(shared).local_network_applied);
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(8);
        while deliveries.lock().unwrap().len() < 3 && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        let seen = deliveries.lock().unwrap().clone();
        assert_eq!(seen.len(), 3, "the refused 'off' is sent again: {seen:?}");
        assert_eq!(seen[2], json!({"localNetwork":true,"generation":2}));
        let applied = async { super::super::commands::view(shared).local_network_applied };
        let mut ok = applied.await;
        for _ in 0..20 {
            if ok {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            ok = super::super::commands::view(shared).local_network_applied;
        }
        assert!(ok, "acknowledged: applied");
        core.shutdown().await;
    }
}

#[cfg(all(test, feature = "your-firefox"))]
mod local_document_tests {
    use super::*;
    #[tokio::test]
    async fn firefox_opens_only_a_confined_passive_snapshot_of_the_selected_file() {
        let f = crate::feature_test_core::Fixture::new().await;
        f.load("local", &f.root, &[], false);
        f.core.shared.access.observe_request(
            "session/agent/update",
            "local",
            &json!({"agentName":"accept-edits"}),
        );
        f.core
            .shared
            .chrome_control
            .tasks
            .lock()
            .unwrap()
            .insert("owned".into(), "local".into());
        f.core
            .shared
            .chrome_control
            .chat_profiles
            .lock()
            .unwrap()
            .insert("local".into(), "firefox:agent".into());
        let path = f.root.join("article.html");
        std::fs::write(
            &path,
            "<h1>Local article</h1><script>document.title='should not run'</script>",
        )
        .unwrap();
        let mut source = url::Url::from_file_path(path).unwrap();
        source.set_fragment(Some("section-2"));
        let mut request = json!({"sessionId":"owned","action":"page.navigate","args":{"tabId":1,"url":source.as_str()}});
        prepare(&f.core.shared, "local", &mut request)
            .await
            .unwrap();
        assert!(
            request["args"]["url"]
                .as_str()
                .unwrap()
                .starts_with("http://127.0.0.1:"),
            "{request}"
        );
        let url = request["args"]["url"].as_str().unwrap();
        assert_eq!(url::Url::parse(url).unwrap().fragment(), Some("section-2"));
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(std::time::Duration::from_secs(3))
            .build()
            .unwrap();
        let response = client.get(url).send().await.unwrap();
        assert!(
            response.headers()["content-security-policy"]
                .to_str()
                .unwrap()
                .contains("script-src 'none'")
        );
        assert!(response.text().await.unwrap().contains("Local article"));
        let sibling = url::Url::parse(url).unwrap().join("sibling.html").unwrap();
        assert_eq!(
            client.get(sibling).send().await.unwrap().status(),
            reqwest::StatusCode::NOT_FOUND
        );
        f.core.shared.chrome_control.stop_chat("local");
        if let Ok(response) = client.get(url).send().await {
            assert!(
                !response.status().is_success(),
                "Stop must revoke a kept-alive preview connection"
            );
        }
        f.finish().await;
    }
}
