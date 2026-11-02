//! The agent's network rule in the person's Chrome: the built-in browser's navigation policy for the agent
//! (the agent column of the navigation policy table), with one Chrome-only switch, Settings > Developer > Local network
//! pages for the agent. Depends on vw-preview (`UrlPolicy`, name checks), `preview_ports` (the chat's proven ports)
//! and vw-netguard (address classes). Called by `escalation` before every call reaches the helper.
//! Invariants:
//! - Every `chrome_execute` call carries `_netPolicy` (`localNetwork`, `loopbackPorts`); the extension judges every
//!   frame, navigation, redirect and request of the agent's tabs with it (extension `netguard.ts`). The model
//!   never supplies it: the gateway refuses `_` arguments.
//! - A `url` argument is checked here first, with DNS (fail closed), exactly as the built-in browser checks the
//!   agent's opens: public hosts; this chat's proven ports on 127.0.0.1, `[::1]` or localhost; private, CGNAT and ULA
//!   addresses (and names resolving there) only with the switch on; link-local, metadata, reserved, other loopback
//!   spellings, other sessions', other apps' and Vibe Work's own ports, and names resolving to this computer, never.
//! - The built-in browser does not read the switch: it keeps refusing local-network pages for the agent.
use crate::core::Shared;
use serde_json::{Value, json};
use std::future::Future;
use std::sync::Arc;
use vw_preview::{Destination, NameVerdict, PortOwner, UrlPolicy, UrlRefusal};

/// Actions whose `url` argument opens or fetches that address.
const URL_ACTIONS: [&str; 4] = [
    "tabs.create",
    "page.navigate",
    "files.download",
    "auth.basic",
];

/// The rule sent with one call.
pub(super) fn policy(local_network: bool, loopback_ports: &[u16]) -> Value {
    json!({"localNetwork": local_network, "loopbackPorts": loopback_ports})
}

/// Checks the call's `url` argument, if its action has one.
///
/// # Errors
/// The refusal the agent reads, naming the address class or the owner of the port.
pub(super) async fn check_url<F, Fut>(
    url_policy: &UrlPolicy,
    args: &Value,
    local_network: bool,
    owner_of: impl FnOnce(u16) -> PortOwner,
    names: F,
) -> Result<(), String>
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = NameVerdict>,
{
    let action = args["action"].as_str().unwrap_or_default();
    let Some(text) = args["args"]["url"]
        .as_str()
        .filter(|_| URL_ACTIONS.contains(&action))
    else {
        return Ok(());
    };
    let refuse = |why: String| format!("Not run: {why}.");
    let (url, destination) = url_policy
        .check(text, owner_of, local_network)
        .map_err(|e| refuse(e.to_string()))?;
    if destination != Destination::Public {
        return Ok(());
    }
    let Some(url::Host::Domain(name)) = url.host() else {
        return Ok(());
    };
    match names(name.to_owned()).await {
        NameVerdict::Public => Ok(()),
        NameVerdict::LocalNetwork(_) if local_network => Ok(()),
        NameVerdict::LocalNetwork(_) => Err(refuse(
            UrlRefusal::Lan(name.chars().take(80).collect()).to_string(),
        )),
        NameVerdict::Refused(why) => Err(refuse(format!("{why}: not opened"))),
    }
}

/// Adds the chat's rule to a `chrome_execute` call and checks its `url` argument (DNS through the preview
/// service's shared lookup; without a preview service names are refused: fail closed).
///
/// # Errors
/// The refusal text.
pub(super) async fn prepare(
    shared: &Arc<Shared>,
    session: &str,
    args: &mut Value,
) -> Result<(), String> {
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
    let preview = shared.preview.clone();
    check_url(
        url_policy,
        args,
        local_network,
        |port| crate::preview_ports::port_owner(shared, &ports, session, port, true),
        |name| async move {
            match preview {
                Some(hub) => hub.service.check_name(&name).await,
                None => NameVerdict::Refused(format!("could not check where {name} points")),
            }
        },
    )
    .await?;
    args["_netPolicy"] = policy(local_network, &ports.allowed(true));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(action: &str, url: &str) -> Value {
        json!({"sessionId":"s","action":action,"args":{"url":url}})
    }
    fn owner(port: u16) -> PortOwner {
        match port {
            5173 => PortOwner::ThisSession,
            8787 => PortOwner::OtherApp("Vibe Work (Debug API)".into()),
            3000 => PortOwner::OtherSession("another session ('Shop')".into()),
            _ => PortOwner::OtherApp("another app (node)".into()),
        }
    }
    async fn names(name: String) -> NameVerdict {
        match name.as_str() {
            "nas.example" => NameVerdict::LocalNetwork(format!("{name} resolves to 10.1.1.20")),
            "rebind.example" => {
                NameVerdict::Refused(format!("{name} resolves to 127.0.0.1, this computer"))
            }
            _ => NameVerdict::Public,
        }
    }
    async fn decide(action: &str, url: &str, lan: bool) -> Result<(), String> {
        check_url(&UrlPolicy::default(), &call(action, url), lan, owner, names).await
    }

    #[tokio::test]
    async fn by_default_each_local_and_special_class_is_refused_before_chrome() {
        for action in URL_ACTIONS {
            for url in [
                "http://10.20.30.6/",
                "http://10.0.0.8:8080/x",
                "http://100.64.0.9/",
                "http://[fd00::5]/",
                "http://[::ffff:172.20.1.1]/",
                "https://nas.example/",
            ] {
                let refused = decide(action, url, false).await.unwrap_err();
                assert!(
                    refused.contains("on your local network"),
                    "{action} {url}: {refused}"
                );
            }
            for url in [
                "http://169.254.169.254/latest/meta-data/",
                "http://[fe80::1]/",
                "http://0.0.0.0/",
                "http://127.0.0.2:5173/",
                "http://app.localhost:5173/",
                "http://127.0.0.1:9999/",
                "http://127.0.0.1:8787/",
                "http://localhost:3000/",
                "https://rebind.example/",
                "file:///etc/passwd",
            ] {
                let refused = decide(action, url, false).await.unwrap_err();
                assert!(
                    refused.starts_with("Not run: "),
                    "{action} {url}: {refused}"
                );
                assert!(
                    !refused.contains("on your local network"),
                    "{action} {url}: {refused}"
                );
            }
            for url in [
                "https://example.com/",
                "http://127.0.0.1:5173/",
                "http://localhost:5173/",
                "https://93.184.216.34/",
            ] {
                decide(action, url, false)
                    .await
                    .unwrap_or_else(|e| panic!("{action} {url}: {e}"));
            }
        }
    }

    #[tokio::test]
    async fn the_switch_opens_local_network_pages_only() {
        for url in [
            "http://10.20.30.6/",
            "http://10.0.0.8:8080/x",
            "http://100.64.0.9/",
            "http://[fd00::5]/",
            "https://nas.example/",
        ] {
            decide("tabs.create", url, true)
                .await
                .unwrap_or_else(|e| panic!("{url}: {e}"));
        }
        for url in [
            "http://169.254.169.254/",
            "http://[fe80::1]/",
            "http://127.0.0.1:9999/",
            "http://127.0.0.1:8787/",
            "http://localhost:3000/",
            "https://rebind.example/",
        ] {
            assert!(decide("tabs.create", url, true).await.is_err(), "{url}");
        }
    }

    /// PREDICT: without the rule wired into `escalation`, the call reaches the (absent) helper and answers
    /// "Your Chrome is not running"; with it, the gateway refuses the address first.
    #[tokio::test]
    async fn the_gateway_refuses_a_local_network_open_before_the_helper_is_asked() {
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
        let open = |url: &str| json!({"sessionId":"t","action":"tabs.create","args":{"url":url}});
        let refused =
            super::super::gateway::call(shared, "s", "chrome_execute", open("http://10.20.30.6/"))
                .await
                .unwrap_err();
        assert!(refused.contains("on your local network"), "{refused}");
        let metadata = super::super::gateway::call(
            shared,
            "s",
            "chrome_execute",
            open("http://169.254.169.254/"),
        )
        .await
        .unwrap_err();
        assert!(metadata.contains("link-local"), "{metadata}");
        shared.chrome_control.set_local_network(true).unwrap();
        // With the switch on the address passes the app's check and goes on to the helper (absent here).
        let passed =
            super::super::gateway::call(shared, "s", "chrome_execute", open("http://10.20.30.6/"))
                .await
                .unwrap();
        assert!(passed.to_string().contains("not running"), "{passed}");
        assert!(
            super::super::gateway::call(
                shared,
                "s",
                "chrome_execute",
                open("http://169.254.169.254/")
            )
            .await
            .is_err()
        );
        // The built-in browser does not read the switch.
        let builtin =
            UrlPolicy::default().check("http://10.20.30.6/", |_| PortOwner::Nobody, false);
        assert!(matches!(builtin, Err(UrlRefusal::Lan(_))));
        core.shutdown().await;
    }

    #[tokio::test]
    async fn actions_without_a_url_argument_are_judged_in_chrome() {
        let read = json!({"sessionId":"s","action":"page.read","args":{"url":"http://10.1.1.1/"}});
        assert!(
            check_url(&UrlPolicy::default(), &read, false, owner, names)
                .await
                .is_ok()
        );
        assert_eq!(
            policy(true, &[5173]),
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
        let _seen = shared.chrome_control.owner.answering(move |_, args| {
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
            shared.chrome_control.set_local_network(on).unwrap();
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
        assert_eq!(seen[2], json!({"localNetwork":false,"generation":2}));
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
