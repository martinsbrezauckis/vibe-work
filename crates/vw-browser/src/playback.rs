//! Bounded page media commands shared with the external-browser compiler; the driver polls playback and seeks.
//! Uses isolated page evaluation and the existing mount/Stop fences. Receipts contain the actual media state.
use serde_json::{Value, json};

use crate::Actor;
use crate::driver::{ActError, Browser, failed, refused};

/// A validated named media operation. No caller-supplied script is accepted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    action: String,
    args: Value,
}

impl Request {
    /// Validate media action arguments before any page access.
    /// # Errors
    /// Unknown actions, missing handles or out-of-range fields.
    pub fn parse(args: &Value) -> Result<Self, String> {
        let action = args["action"].as_str().unwrap_or_default();
        if !matches!(
            action,
            "media.read"
                | "media.play"
                | "media.pause"
                | "media.seek"
                | "media.mute"
                | "media.transcript"
        ) {
            return Err(
                "Unknown media action; use media.read, play, pause, seek, mute or transcript."
                    .into(),
            );
        }
        let mut input = json!({"action":action});
        if action != "media.read" {
            let id = args["mediaId"]
                .as_str()
                .filter(|id| {
                    id.len() <= 64
                        && id.starts_with('m')
                        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                })
                .ok_or("Media control needs a mediaId from media.read; read the media first.")?;
            input["mediaId"] = json!(id);
        }
        if action == "media.seek" {
            input["timeMs"] = json!(args["timeMs"].as_u64().filter(|n| *n <= 604_800_000).ok_or("Media seek needs timeMs from 0 to 604800000; choose a position within the media duration.")?);
        }
        if action == "media.mute" {
            input["muted"] = json!(
                args["muted"]
                    .as_bool()
                    .ok_or("Media mute needs muted: true or false.")?
            );
        }
        Ok(Self {
            action: action.into(),
            args: input,
        })
    }

    /// Shared action name used by permission classification and receipts.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.action
    }

    /// Fixed isolated-world script for this operation, or its pending receipt.
    #[must_use]
    pub fn script(&self, token: Option<&str>) -> String {
        let args = token.map_or_else(
            || self.args.clone(),
            |token| json!({"action":"media.poll","token":token}),
        );
        include_str!("js/playback.js").replace("__VW_MEDIA_REQUEST__", &args.to_string())
    }
}

impl Browser {
    pub(crate) async fn playback(
        &self,
        session: &str,
        request: &Request,
        by: Actor,
    ) -> Result<Value, ActError> {
        let bound = self.bind_read(session, by).await.map_err(refused)?;
        let mount = bound
            .read_mount
            .as_ref()
            .ok_or_else(|| refused(crate::NOTHING_OPEN))?;
        let url = self
            .service
            .view(session)
            .and_then(|v| v.url)
            .ok_or_else(|| refused(crate::NOTHING_OPEN))?;
        let page = bound.page_of(session).await.map_err(refused)?;
        let mut token = None;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            bound.check_answer(session, &page, &url).map_err(refused)?;
            let text = self
                .service
                .evaluate_on_mount_checked(
                    mount,
                    request.script(token.as_deref()),
                    self.dispatch_permission_check(session, by),
                )
                .await
                .map_err(|e| failed(crate::driver::preview_text(&e)))?;
            bound.check_read_mount().map_err(refused)?;
            let data: Value = crate::model::parse_answer(&text).map_err(failed)?;
            if let Some(error) = data["error"].as_str() {
                return Err(failed(error));
            }
            if data["pending"] != true {
                return Ok(
                    json!({"ok":true,"Outcome":"Media state read back.","action":request.name(),"untrustedPageContent":data}),
                );
            }
            if token.is_none() {
                token = data["token"].as_str().map(str::to_owned);
            }
            if token.is_none() || tokio::time::Instant::now() >= deadline {
                return Err(failed(
                    "Media operation did not settle within five seconds; read media state before deciding whether to try again.",
                ));
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    }
}
