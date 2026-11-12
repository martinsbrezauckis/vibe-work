//! Private, one-operation Safari app services. The helper supplies immutable authority; Stop drops pending OS waits.
use crate::core::Shared;
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
#[path = "safari_paste.rs"]
mod paste;
#[derive(Debug)]
struct ServiceFault {
    code: &'static str,
    message: String,
}
impl From<String> for ServiceFault {
    fn from(message: String) -> Self {
        Self {
            code: "SAFARI_APP_REFUSED",
            message,
        }
    }
}
impl From<&str> for ServiceFault {
    fn from(message: &str) -> Self {
        message.to_owned().into()
    }
}
fn after_request(claim: &Value, message: String) -> ServiceFault {
    if (claim["kind"] == "native" && claim["stage"] == "dispatch")
        || (claim["kind"] == "clipboard"
            && matches!(
                claim["action"].as_str(),
                Some("clipboard.write" | "clipboard.write_rich")
            ))
    {
        ServiceFault {
            code: "OUTCOME_UNKNOWN",
            message: format!(
                "{message} The requested effect may have occurred. Inspect the page or clipboard before continuing; do not repeat the action without checking."
            ),
        }
    } else {
        message.into()
    }
}
struct Cancel(Arc<AtomicBool>);
impl Drop for Cancel {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}
pub(super) fn observe(shared: &Arc<Shared>, params: &Value) {
    let Some(id) = params["requestId"].as_str().filter(|id| id.len() <= 128) else {
        return;
    };
    let shared = shared.clone();
    let id = id.to_owned();
    tokio::spawn(async move {
        let Some(claim) = shared
            .chrome_control
            .owner
            .rpc("owner.safariService.claim", json!({"requestId":id}))
            .await
        else {
            return;
        };
        let result = run(&shared, &claim).await;
        let response = match result {
            Ok(data) => json!({"requestId":id,"data":data}),
            Err(error) => {
                json!({"requestId":id,"error":{"code":error.code,"message":error.message}})
            }
        };
        let _ = shared
            .chrome_control
            .owner
            .rpc("owner.safariService.result", response)
            .await;
    });
}
fn authority(shared: &Shared, claim: &Value) -> Result<(), String> {
    let chat = claim["clientSessionId"]
        .as_str()
        .ok_or("Safari supplied no chat binding. Start a new session.")?;
    if !claim["profileId"]
        .as_str()
        .is_some_and(|p| p.starts_with("safari:"))
        || !super::safari::enabled(shared)
    {
        return Err(
            "Safari is no longer enabled for this operation. Reconnect Safari before continuing."
                .into(),
        );
    }
    let request = json!({"sessionId":claim["sessionId"],"profileId":claim["profileId"],"action":claim["action"],"args":claim["args"]});
    let needed = super::mapping::required("chrome_execute", &request).ok_or(
        "This Safari action has no shared permission classification. Use an advertised action.",
    )?;
    // A private owner claim is issued only for this exact already-approved operation. It does not authorize another action.
    super::check::authorize_read(
        shared,
        chat,
        "chrome_execute",
        &request,
        Some(&super::read_consent::Consent(needed)),
    )?;
    super::gateway::validate_session(shared, chat, "chrome_execute", &request)?;
    super::reconnect::stopped(shared, chat, super::agent_browser::Browser::Safari)
}
fn clipboard_wire(mut data: Value) -> Value {
    if let Some(image) = data
        .as_object_mut()
        .and_then(|object| object.remove(vw_agent_tools::wire::IMAGE_KEY))
    {
        data["mimeType"] = image["mimeType"].clone();
        data["dataUrl"] = json!(format!(
            "data:{};base64,{}",
            image["mimeType"].as_str().unwrap_or_default(),
            image["data"].as_str().unwrap_or_default()
        ));
    }
    data
}
async fn run(shared: &Arc<Shared>, claim: &Value) -> Result<Value, ServiceFault> {
    authority(shared, claim)?;
    let cancel = Cancel(Arc::new(AtomicBool::new(false)));
    let work = async {
        match claim["kind"].as_str() {
            Some("native") => {
                let mut prepared = claim["prepared"].clone();
                if claim["stage"] == "dispatch"
                    && claim["action"] == "page.key"
                    && super::mapping::clipboard_shortcut(&claim["args"])
                        == Some(vw_ask::chrome::ClipboardShortcut::Paste)
                {
                    let revision = vw_platform::safari_input::clipboard_revision().await?;
                    paste::check(shared).await?;
                    prepared["clipboardRevision"] = json!(revision);
                }
                vw_platform::safari_input::request(
                    claim["stage"].as_str().unwrap_or_default(),
                    prepared,
                    claim["action"].as_str().unwrap_or_default(),
                    claim["args"].clone(),
                    claim["target"]["url"].as_str().unwrap_or_default(),
                    cancel.0.clone(),
                )
                .await
                .map_err(|message| after_request(claim, message))
            }
            Some("clipboard") => {
                let mut args = claim["args"].clone();
                args["action"] = claim["action"].clone();
                crate::browser_clipboard::run(shared, &args)
                    .await
                    .map(clipboard_wire)
                    .map_err(|message| after_request(claim, message))
            }
            _ => Err("The Safari app service is unknown. Use an advertised action.".into()),
        }
    };
    tokio::pin!(work);
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(8);
    loop {
        tokio::select! {
            result=&mut work=>{authority(shared,claim).map_err(|message| after_request(claim, message))?;return result;},
            ()=tokio::time::sleep(std::time::Duration::from_millis(50))=>{
                authority(shared,claim).map_err(|message| after_request(claim, message))?;
                if tokio::time::Instant::now()>=deadline{return Err(after_request(claim,"The Safari app service did not finish before its deadline. Inspect the page or clipboard before continuing.".into()));}
                let live=tokio::time::timeout(std::time::Duration::from_millis(250),shared.chrome_control.owner.rpc("owner.safariService.check",json!({"requestId":claim["requestId"]}))).await;
                if !matches!(live,Ok(Some(ref v)) if v["active"]==true){return Err(after_request(claim,"Safari's operation, connection or permission ended. Read the current session before continuing.".into()));}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn app_service_dispatch_failures_preserve_uncertain_effects() {
        for claim in [
            json!({"kind":"native","stage":"dispatch"}),
            json!({"kind":"clipboard","action":"clipboard.write"}),
            json!({"kind":"clipboard","action":"clipboard.write_rich"}),
        ] {
            assert_eq!(
                after_request(&claim, "lost confirmation".into()).code,
                "OUTCOME_UNKNOWN"
            );
        }
        for claim in [
            json!({"kind":"native","stage":"prepare"}),
            json!({"kind":"clipboard","action":"clipboard.read"}),
        ] {
            assert_eq!(
                after_request(&claim, "refused before input".into()).code,
                "SAFARI_APP_REFUSED"
            );
        }
        assert_eq!(
            ServiceFault::from("vault refusal before dispatch").code,
            "SAFARI_APP_REFUSED"
        );
    }
    #[tokio::test]
    async fn app_service_rechecks_ownership_mode_and_enabled_state() {
        let core =
            crate::offline_core::start_with("safari-app-service-authority", |config, dir| {
                config.chrome_control.package = Some(dir.join("package"));
            })
            .await;
        let shared = &core.shared;
        assert!(
            shared
                .store
                .load(
                    json!({"session":{"id":"chat"},"history":[],"turns":[]}),
                    vw_session::LoadKind::New
                )
                .is_ok()
        );
        shared.access.observe_request(
            "session/agent/update",
            "chat",
            &json!({"agentName":"auto-approve"}),
        );
        let dir = shared.chrome_control.dir().unwrap();
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("safari-enabled.json"), b"true").unwrap();
        shared
            .chrome_control
            .tasks
            .lock()
            .unwrap()
            .insert("task".into(), "chat".into());
        shared
            .chrome_control
            .work
            .lock()
            .unwrap()
            .open("task", "chat", "active", "safari:p", None);
        let mut claim = json!({"clientSessionId":"chat","sessionId":"task","profileId":"safari:p","action":"page.click","args":{"ref":"r"}});
        assert!(authority(shared, &claim).is_ok());
        claim["clientSessionId"] = json!("other");
        assert!(authority(shared, &claim).is_err());
        claim["clientSessionId"] = json!("chat");
        claim["action"] = json!("clipboard.read");
        assert!(authority(shared, &claim).is_ok());
        shared
            .access
            .observe_request("session/agent/update", "chat", &json!({"agentName":"ask"}));
        assert!(authority(shared, &claim).unwrap_err().contains("Full"));
        std::fs::write(dir.join("safari-enabled.json"), b"false").unwrap();
        claim["action"] = json!("page.click");
        assert!(authority(shared, &claim).is_err());
        core.shutdown().await;
    }
    #[test]
    fn app_clipboard_image_uses_the_helpers_existing_image_receipt() {
        let data = clipboard_wire(
            json!({"readBack":true,vw_agent_tools::wire::IMAGE_KEY:{"mimeType":"image/png","data":"YWJj"}}),
        );
        assert_eq!(data["mimeType"], "image/png");
        assert_eq!(data["dataUrl"], "data:image/png;base64,YWJj");
        assert!(data.get(vw_agent_tools::wire::IMAGE_KEY).is_none());
    }
    #[test]
    fn dropping_an_app_wait_revokes_native_worker_effects() {
        let flag = Arc::new(AtomicBool::new(false));
        {
            let _guard = Cancel(flag.clone());
            assert!(!flag.load(Ordering::Acquire));
        }
        assert!(flag.load(Ordering::Acquire));
    }
}

#[cfg(test)]
#[path = "safari_service_cancel_tests.rs"]
mod cancel_tests;
