//! Vault login fill through the checked Chrome owner. The model supplies only a ref and login name.
//! The helper binds the inspected document/revision; `browser_fill` owns all vault decisions.
use super::{escalation, owner};
use crate::ask::tool_calls::receipts::Execution;
use crate::{
    browser_fill::{self, LoginWriter},
    core::Shared,
};
use serde_json::{Value, json};
use std::sync::{Arc, Weak};
use vw_browser::{FillTarget, LoginRole};
use vw_gate::Requester;

struct ChromeLoginWriter {
    shared: Weak<Shared>,
    session: String,
    args: Value,
}
impl LoginWriter for ChromeLoginWriter {
    async fn write(
        &self,
        _target: &FillTarget,
        login: &vw_keys::Login,
        value: &str,
    ) -> Result<Value, String> {
        let shared = self.shared.upgrade().ok_or("Vibe Work closed.")?;
        let session = self.session.as_str();
        super::gateway::validate_session(&shared, session, "chrome_execute", &self.args)?;
        crate::agent_access::check_for(&shared, browser_fill::METHOD, session)?;
        if shared.store.view(session).is_none()
            || super::check::authorize(&shared, session, "chrome_execute", &self.args).is_err()
        {
            return Err(
                "Not filled: this chat closed or Chrome login access was turned off.".to_owned(),
            );
        }
        let mut args = self.args.clone();
        args["_loginValue"] = if args["action"] == "auth.basic" {
            json!(json!({"username":login.username,"password":value}).to_string())
        } else {
            json!(value)
        };
        args["_approvedCapabilities"] = json!(["auth.secrets"]);
        // The write is checked in Chrome under the chat's network rule like every other call (escalation::send):
        // without it, Chrome judged a sign-in page on the local network with the switch off.
        super::network::prepare(&shared, session, &mut args).await?;
        // Only the helper's answer can be a fill receipt; the core's own failure (connection lost) is no fill.
        let owner::Answer::Helper(result) = shared
            .chrome_control
            .owner
            .call("chrome_execute", args)
            .await
        else {
            return Err(
                "Login fill did not complete: Chrome did not confirm a filled field. Check Chrome before trying again.".to_owned(),
            );
        };
        if result["structuredContent"]["status"] != "completed"
            || result["structuredContent"]["data"]["filled"] != true
        {
            // Diagnostics only: the helper's status and error code (never page words, a value or the password).
            let code = result["structuredContent"]["error"]["code"]
                .as_str()
                .filter(|c| {
                    c.len() <= 40 && c.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                });
            tracing::warn!(
                status = result["structuredContent"]["status"]
                    .as_str()
                    .unwrap_or("none"),
                code = code.unwrap_or("none"),
                "Chrome login fill did not complete"
            );
            return Err(
                "Login fill did not complete: Chrome did not confirm a filled field. Check Chrome before trying again.".to_owned(),
            );
        }
        Ok(result)
    }
}

pub(super) async fn send(
    shared: &Arc<Shared>,
    session: &str,
    tool: &str,
    args: Value,
) -> Result<owner::Answer, String> {
    if tool == "chrome_execute"
        && [json!("auth.fill"), json!("auth.basic")].contains(&args["action"])
    {
        Ok(owner::own(
            "Not filled: vault consent must be settled before the tool starts.",
            "VAULT_PREPARATION_REQUIRED",
        ))
    } else {
        escalation::send(shared, session, tool, args).await
    }
}

pub(crate) async fn prepare(
    shared: &Arc<Shared>,
    session: &str,
    mut args: Value,
) -> Result<Execution, String> {
    super::gateway::validate_public_args(&args)?;
    // The mode refusal comes first, as in the gateway: the agent learns which mode the action needs.
    super::check::authorize(shared, session, "chrome_execute", &args)?;
    super::gateway::validate_session(shared, session, "chrome_execute", &args)?;
    let checked = escalation::send(shared, session, "chrome_execute", args.clone()).await?;
    let vault = matches!(&checked, owner::Answer::Helper(result)
        if result["structuredContent"]["status"] == "failed"
            && result["structuredContent"]["error"]["code"] == "VAULT_REQUIRED"
            && result["structuredContent"]["error"]["details"]["noEffect"] == true);
    if !vault {
        let mut answer = super::content::for_model(checked);
        super::output::guard(&mut answer);
        return Ok(Box::pin(std::future::ready(Ok(
            json!({"vwMcpResult":answer}),
        ))));
    }
    let body = &checked.value()["structuredContent"];
    let target = &body["error"]["details"]["target"];
    let origin = target["origin"]
        .as_str()
        .ok_or("Not filled: no checked login origin.")?;
    let parsed = url::Url::parse(origin).map_err(|_| "Not filled: invalid login origin.")?;
    let role = match target["role"].as_str() {
        Some("password") => LoginRole::Password,
        Some("username") => LoginRole::Username,
        _ => return Err("Not filled: no checked login role.".to_owned()),
    };
    let target = FillTarget {
        reference: args["args"][if args["action"] == "auth.basic" {
            "url"
        } else {
            "ref"
        }]
        .as_str()
        .ok_or("Not filled: no field ref.")?
        .to_owned(),
        role,
        origin: origin.to_owned(),
        host: parsed
            .host_str()
            .ok_or("Not filled: no login host.")?
            .to_owned(),
        label: if args["action"] == "auth.basic" {
            "HTTP sign-in"
        } else {
            role.as_str()
        }
        .to_owned(),
    };
    let wanted = args["args"]["login"]
        .as_str()
        .ok_or("Not filled: name a vault login.")?
        .to_owned();
    args["_preflightOperationId"] = body["operationId"].clone();
    let writer = ChromeLoginWriter {
        shared: Arc::downgrade(shared),
        session: session.to_owned(),
        args,
    };
    let current_args = writer.args.clone();
    let current_session = session.to_owned();
    let current_shared = Arc::downgrade(shared);
    let execution =
        browser_fill::prepare_checked(shared, session, &wanted, target, Requester::Agent, writer)
            .await?;
    Ok(Box::pin(async move {
        let shared = current_shared.upgrade().ok_or("Vibe Work closed.")?;
        super::gateway::validate_session(
            &shared,
            &current_session,
            "chrome_execute",
            &current_args,
        )?;
        super::check::authorize(&shared, &current_session, "chrome_execute", &current_args)?;
        let mut receipt = execution.await?;
        super::content::mark(&mut receipt);
        super::output::guard(&mut receipt);
        Ok(json!({"vwMcpResult":receipt}))
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn running_vault_call_returns_without_starting_consent() {
        let core = crate::offline_core::start("chrome-vault-hook").await;
        for action in ["auth.fill", "auth.basic"] {
            let result = tokio::time::timeout(
                std::time::Duration::from_millis(100),
                send(
                    &core.shared,
                    "s",
                    "chrome_execute",
                    json!({"action":action,"args":{"ref":"field","login":"fixture"}}),
                ),
            )
            .await
            .unwrap()
            .unwrap();
            // The core's own refusal: never wrapped as page content.
            let owner::Answer::Own(result) = result else {
                panic!("the vault preparation refusal is the core's own")
            };
            assert_eq!(
                result["structuredContent"]["error"]["code"],
                "VAULT_PREPARATION_REQUIRED"
            );
        }
        core.shutdown().await;
    }

    /// Seen in a survey run: with the local-network switch on, every vault fill on a LAN sign-in page failed "Login
    /// fill did not complete": the consent was given, but the write reached Chrome without the chat's network rule,
    /// so Chrome refused it as `LOCAL_NETWORK`. PREDICT (before the change): the write carries no `_netPolicy`.
    #[tokio::test]
    async fn the_fill_write_carries_the_chats_network_rule() {
        let core = crate::offline_core::start_with("chrome-fill-net", |config, dir| {
            config.chrome_control.package = Some(dir.join("chrome-package"));
        })
        .await;
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
            &json!({"agentName":"auto-approve"}),
        );
        shared
            .chrome_control
            .tasks
            .lock()
            .unwrap()
            .insert("t".into(), "s".into());
        shared.chrome_control.set_local_network(true);
        let seen = super::super::test_support::answering(shared, |_, _| {
            json!({"isError":false,"content":[{"type":"text","text":"filled"}],
                "structuredContent":{"operationId":"op","status":"completed","data":{"filled":true}}})
        });
        let writer = ChromeLoginWriter {
            shared: Arc::downgrade(shared),
            session: "s".into(),
            args: json!({"sessionId":"t","action":"auth.fill","args":{"ref":"field","login":"Demo"}}),
        };
        let target = FillTarget {
            reference: "field".into(),
            role: LoginRole::Username,
            origin: "https://10.20.30.6:4443".into(),
            host: "10.20.30.6".into(),
            label: "username".into(),
        };
        let login = vw_keys::Login {
            id: "vibe-work-login-0123456789abcdef".into(),
            name: "Demo".into(),
            url: "https://10.20.30.6:4443/login".into(),
            origin: "https://10.20.30.6:4443".into(),
            host: "10.20.30.6".into(),
            username: "survey-user".into(),
            has_password: true,
            own: true,
            updated_ms: 0,
        };
        writer.write(&target, &login, "survey-user").await.unwrap();
        let sent = seen
            .lock()
            .unwrap()
            .iter()
            .find(|(tool, _)| tool == "chrome_execute")
            .cloned()
            .unwrap();
        assert_eq!(sent.1["_netPolicy"]["localNetwork"], true, "{}", sent.1);
        core.shutdown().await;
    }
}
