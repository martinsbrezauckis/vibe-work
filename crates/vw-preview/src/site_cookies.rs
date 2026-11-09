//! Current-site cookie requests shared by browser tools and native engines.
//! The service pins the mount and URL; engine implementations must recheck the URL before accessing its store.
use serde::Deserialize;
use serde_json::Value;
use url::Url;

use crate::{EngineError, MountId, PreviewError, PreviewService};

/// A bounded cookie action. Domain is always derived from the mounted page, never from tool arguments.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct CookieRequest {
    action: String,
    #[serde(default)]
    names: Option<Vec<String>>,
    #[serde(default)]
    operation: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    value: String,
    #[serde(default = "root_path")]
    path: String,
    #[serde(default)]
    secure: bool,
    #[serde(default, rename = "httpOnly")]
    http_only: bool,
    #[serde(default, rename = "sameSite")]
    same_site: Option<String>,
}

fn root_path() -> String {
    "/".into()
}

impl CookieRequest {
    /// Validate a browser tool's cookie request before engine dispatch.
    ///
    /// # Errors
    /// Unknown operations, wrong field types or unbounded names/values.
    pub fn parse(args: &Value) -> Result<Self, String> {
        let request: Self = serde_json::from_value(args.clone())
            .map_err(|_| "Invalid cookie arguments; use the advertised fields.")?;
        if !matches!(
            request.action.as_str(),
            "cookies.read" | "cookies.metadata" | "cookies.change"
        ) {
            return Err("Unknown cookie action.".into());
        }
        if request
            .names
            .as_ref()
            .is_some_and(|names| names.len() > 100 || names.iter().any(|n| n.len() > 256))
        {
            return Err("Read at most 100 cookie names of at most 256 bytes each.".into());
        }
        if request.action == "cookies.change" {
            if !matches!(request.operation.as_str(), "set" | "remove" | "clear") {
                return Err("Unknown cookie operation; use set, remove or clear.".into());
            }
            if request.operation != "clear"
                && (request.name.is_empty()
                    || request.name.len() > 256
                    || request
                        .name
                        .chars()
                        .any(|c| c.is_control() || matches!(c, ';' | '=')))
            {
                return Err("Cookie change needs a valid name of at most 256 bytes.".into());
            }
            if request.operation == "set"
                && (!args["value"].is_string()
                    || request.value.len() > 4096
                    || request.value.chars().any(|c| c.is_control() || c == ';'))
            {
                return Err("Cookie value must be text of at most 4096 bytes without control characters or semicolons.".into());
            }
            if !request.path.starts_with('/')
                || request.path.starts_with("//")
                || request.path.len() > 1000
                || request.path.chars().any(|c| c.is_control() || c == ';')
            {
                return Err("Cookie path must be an absolute path on this site.".into());
            }
            if request
                .same_site
                .as_deref()
                .is_some_and(|s| !matches!(s, "no_restriction" | "lax" | "strict" | "unspecified"))
            {
                return Err(
                    "Invalid sameSite; use no_restriction, lax, strict or unspecified.".into(),
                );
            }
        }
        Ok(request)
    }
    /// Shared action name.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self.action.as_str() {
            "cookies.read" => "cookies.read",
            "cookies.change" => "cookies.change",
            _ => "cookies.metadata",
        }
    }
    /// Validated native adapter parameters, without a caller-controlled domain or URL.
    #[must_use]
    pub fn parameters(&self) -> Value {
        serde_json::json!({"action":self.action,"names":self.names,"operation":self.operation,"name":self.name,"value":self.value,"path":self.path,"secure":self.secure,"httpOnly":self.http_only,"sameSite":self.same_site})
    }
}

impl PreviewService {
    /// Runs a cookie request only on the selected live mount's current HTTP(S) site.
    ///
    /// # Errors
    /// Closed/changed page, non-web origin or native store failure.
    pub async fn site_cookies(
        &self,
        id: &MountId,
        url: Url,
        request: CookieRequest,
    ) -> Result<Value, PreviewError> {
        let _activity = self.session_lock(&id.session_id).await;
        if self.page_mount(&id.session_id).as_ref() != Some(id)
            || self.view(&id.session_id).and_then(|v| v.url).as_deref() != Some(url.as_str())
        {
            return Err(EngineError::NotMounted.into());
        }
        if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
            return Err(EngineError::Unsupported(
                "Cookies need a current HTTP or HTTPS page.".into(),
            )
            .into());
        }
        Ok(self
            .bounded("site cookies", self.engine.site_cookies(id, url, request))
            .await?)
    }
}
