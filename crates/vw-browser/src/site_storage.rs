//! Bounded local/session storage actions in the current document's isolated world.
//! Called by the browser action loop; binds the URL and mount before mutation and scrubs every returned value.
use serde::Deserialize;
use serde_json::{Value, json};

use crate::Actor;
use crate::driver::{ActError, Browser, failed, refused};

/// One current-site storage request, using the shared browser action names.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(tag = "action")]
pub enum Request {
    /// List keys without values.
    #[serde(rename = "storage.metadata")]
    Metadata {
        /// Local or session storage.
        area: String,
    },
    /// Read named keys; absent keys return null.
    #[serde(rename = "storage.read")]
    Read {
        /// Local or session storage.
        area: String,
        /// Exact keys to read.
        keys: Vec<String>,
    },
    /// Set, remove or clear this document's storage area.
    #[serde(rename = "storage.change")]
    Change {
        /// Local or session storage.
        area: String,
        /// Set, remove or clear.
        operation: String,
        /// Exact key, except for clear.
        key: Option<String>,
        /// Replacement text for set.
        value: Option<String>,
    },
}

impl Request {
    pub(crate) fn parse(args: &Value) -> Result<Self, String> {
        let request: Self = serde_json::from_value(args.clone()).map_err(|_| "Storage needs area local/session, and keys for a read or operation/key/value for a change.")?;
        if !matches!(request.area(), "local" | "session") {
            return Err("Unknown storage area; use local or session.".into());
        }
        match &request {
            Self::Read { keys, .. } if keys.len() > 100 || keys.iter().any(|k| k.len() > 4096) => {
                return Err("Read at most 100 storage keys, each at most 4096 bytes.".into());
            }
            Self::Change {
                operation,
                key,
                value,
                ..
            } => {
                if !matches!(operation.as_str(), "set" | "remove" | "clear") {
                    return Err("Unknown storage operation; use set, remove or clear.".into());
                }
                if operation != "clear" && key.as_ref().is_none_or(|k| k.len() > 4096) {
                    return Err("Storage change needs a key of at most 4096 bytes.".into());
                }
                if operation == "set" && value.as_ref().is_none_or(|v| v.len() > 60_000) {
                    return Err("Storage value must be a string of at most 60000 bytes.".into());
                }
            }
            _ => {}
        }
        Ok(request)
    }

    pub(crate) fn name(&self) -> &'static str {
        match self {
            Self::Metadata { .. } => "storage.metadata",
            Self::Read { .. } => "storage.read",
            Self::Change { .. } => "storage.change",
        }
    }

    fn area(&self) -> &str {
        match self {
            Self::Metadata { area } | Self::Read { area, .. } | Self::Change { area, .. } => area,
        }
    }

    fn script(&self, url: &str) -> String {
        let operation = match self {
            Self::Metadata { .. } => "return {keys:Array.from({length:Math.min(store.length,200)},(_,i)=>store.key(i)),truncated:store.length>200}".into(),
            Self::Read { keys, .. } => format!("const values=Object.create(null),tooLargeKeys=[];for(const k of {}){{const v=store.getItem(k);if(v!==null&&v.length>4096)tooLargeKeys.push(k);else values[k]=v;}}return {{values,tooLargeKeys}}", json!(keys)),
            Self::Change { operation, key, value, .. } => match operation.as_str() {
                "set" => format!("store.setItem({},{});return {{key:{},value:store.getItem({})}}",json!(key),json!(value),json!(key),json!(key)),
                "remove" => format!("store.removeItem({});return {{key:{},value:store.getItem({})}}",json!(key),json!(key),json!(key)),
                _ => "store.clear();return {cleared:store.length===0}".into(),
            },
        };
        format!(
            "JSON.stringify((()=>{{if(location.href!=={})return {{error:'The page changed; read it again before accessing storage.'}};try{{const store={};{operation}}}catch{{return {{error:'Page storage access failed; check this site before trying again.'}}}}}})())",
            json!(url),
            if self.area() == "local" {
                "localStorage"
            } else {
                "sessionStorage"
            }
        )
    }
}

impl Browser {
    pub(crate) async fn site_cookies(
        &self,
        session: &str,
        request: &vw_preview::CookieRequest,
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
            .and_then(|u| url::Url::parse(&u).ok())
            .ok_or_else(|| refused(crate::NOTHING_OPEN))?;
        let page = bound.page_of(session).await.map_err(refused)?;
        bound
            .check_answer(session, &page, url.as_str())
            .map_err(refused)?;
        let data = self
            .service
            .site_cookies(mount, url, request.clone())
            .await
            .map_err(|e| failed(crate::driver::preview_text(&e)))?;
        bound.check_read_mount().map_err(refused)?;
        let text = data.to_string();
        let text = vw_secrets::scrub(text.as_bytes())
            .map_or(text, |(v, _)| String::from_utf8_lossy(&v).into_owned());
        let data: Value = serde_json::from_str(&text)
            .map_err(|_| failed("Cookie result could not be redacted."))?;
        Ok(
            json!({"Outcome":"Cookie operation completed.","action":request.name(),"untrustedPageContent":data}),
        )
    }

    pub(crate) async fn site_storage(
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
        bound.check_answer(session, &page, &url).map_err(refused)?;
        let text = self
            .service
            .evaluate_on_mount_checked(
                mount,
                request.script(&url),
                self.dispatch_permission_check(session, by),
            )
            .await
            .map_err(|e| failed(crate::driver::preview_text(&e)))?;
        bound.check_read_mount().map_err(refused)?;
        let text = vw_secrets::scrub(text.as_bytes()).map_or(text.clone(), |(v, _)| {
            String::from_utf8_lossy(&v).into_owned()
        });
        let data: Value = crate::model::parse_answer(&text).map_err(failed)?;
        if let Some(error) = data["error"].as_str() {
            return Err(failed(error));
        }
        Ok(
            json!({"Outcome":"Storage operation completed.","action":request.name(),"untrustedPageContent":data}),
        )
    }
}
