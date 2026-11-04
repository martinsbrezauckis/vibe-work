//! Raw proxy writes retain the original gate consent and Vibe-only routing; the password goes to the vault.
//! Dispatch intercepts writes before transport and masks outward reads. Native dialog text remains unchanged.
use crate::{Command, CommandResult, Core, CoreError, ErrorCode, Origin};
use serde_json::{Value, json};
use vw_gate::{ConfirmationToken, Requester};

pub(crate) fn mask_values(values: &mut Value) {
    for key in ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY"] {
        if let Some(raw) = values.get(key).and_then(Value::as_str) {
            values[key] = vw_mistral::network::form_proxy_url(raw).into();
        }
    }
}

pub(crate) fn mask_reply(method: &str, reply: &mut Value) {
    if matches!(method, "config/proxy/read" | "config/proxy/write")
        && let Some(values) = reply.pointer_mut("/settings/values")
    {
        mask_values(values);
    }
}

impl Core {
    pub(crate) async fn intercept_proxy_write(
        &self,
        origin: Origin,
        command: &Command,
    ) -> Option<Result<CommandResult, CoreError>> {
        if let Command::Call {
            method,
            params,
            confirmation_token,
        } = command
            && method == "config/proxy/write"
        {
            Some(
                self.network_raw_write(origin, params, confirmation_token.clone())
                    .await,
            )
        } else {
            None
        }
    }

    async fn network_raw_write(
        &self,
        origin: Origin,
        params: &Value,
        token: Option<String>,
    ) -> Result<CommandResult, CoreError> {
        let method = "config/proxy/write";
        let map_error = |error| CoreError::from_gate(method, &error);
        let mut token = token.map(ConfirmationToken::from_wire);
        if token.is_none() && origin == Origin::DebugApi {
            token = Some(
                self.shared
                    .gate
                    .request_confirmation_as(method, params, Requester::DebugApi)
                    .await
                    .map_err(map_error)?,
            );
        }
        self.shared
            .gate
            .check(method, params, token.as_ref())
            .map_err(map_error)?;
        let keys = self.shared.config.keys.as_ref().ok_or_else(|| {
            CoreError::new(
                ErrorCode::KeyService,
                "Unlock or set up the vault to save the proxy sign-in.",
            )
        })?;
        let dotenv = self.shared.instructions.own_home.join(".env");
        let requested = params["changes"]
            .as_object()
            .ok_or_else(|| CoreError::bad_result(method, "changes"))?;
        let changes = requested
            .iter()
            .map(|(key, value)| (key.clone(), value.as_str().map(str::to_owned)))
            .collect();
        let settings = keys
            .prepare_vibe_proxy_write(dotenv.clone(), changes)
            .await
            .map_err(|error| CoreError::new(ErrorCode::KeyService, error.to_string()))?;
        let child = keys.child_network_settings(&settings);
        let mut changes: serde_json::Map<String, Value> = requested
            .keys()
            .map(|key| {
                (
                    key.clone(),
                    child.get(key).map_or(Value::Null, Value::String),
                )
            })
            .collect();
        if child.has_proxy() {
            changes.insert(
                "NO_PROXY".into(),
                vw_mistral::network::with_loopback(child.no_proxy.as_deref().unwrap_or_default())
                    .into(),
            );
        }
        let forwarded = json!({"sessionId": params["sessionId"], "changes": changes});
        let token = self
            .shared
            .gate
            .confirm_proxy_forwarding(params, &forwarded, token.as_ref())
            .map_err(map_error)?;
        let mut reply = self
            .forward(origin, method, forwarded, Some(token.as_str().to_owned()))
            .await?;
        keys.finish_vibe_proxy_write(dotenv, settings)
            .await
            .map_err(|error| CoreError::new(ErrorCode::KeyService, error.to_string()))?;
        mask_reply(method, &mut reply.result);
        Ok(CommandResult::Reply {
            result: reply.result,
            client_user_message_id: reply.client_user_message_id,
            user_entry_ids: reply.user_entry_ids,
        })
    }
}
