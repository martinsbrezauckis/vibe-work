//! Bounded native request reads; URLs are stripped of query values and headers default to safe metadata.
//! Shared core classification reserves unredacted headers for Full access; vault-filled documents refuse them.
use crate::{
    ActError, Actor, Browser,
    driver::{failed, refused},
};
use serde::Deserialize;
use serde_json::{Value, json};

/// Request-list filters or one request's header read.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    action: String,
    #[serde(default)]
    offset: usize,
    limit: Option<usize>,
    last: Option<usize>,
    url: Option<String>,
    method: Option<String>,
    status: Option<u16>,
    #[serde(default)]
    failed_only: bool,
    #[serde(rename = "requestId")]
    id: Option<String>,
    #[serde(default)]
    unredacted: bool,
}
impl Request {
    pub(crate) fn parse(args: &Value) -> Result<Self, String> {
        let request:Self=serde_json::from_value(args.clone()).map_err(|_|"Request diagnostics need typed filters; unredacted and failedOnly must be booleans.")?;
        if request.offset > 20000
            || request.limit.is_some_and(|n| !(1..=100).contains(&n))
            || request.last.is_some_and(|n| !(1..=100).contains(&n))
            || request.last.is_some() && args.get("offset").is_some()
            || request.url.as_ref().is_some_and(|s| s.len() > 2000)
            || request.method.as_ref().is_some_and(|s| {
                s.is_empty() || s.len() > 16 || !s.bytes().all(|b| b.is_ascii_alphabetic())
            })
        {
            return Err("Request filters exceed their limits; use offset up to 20000, limit/last 1–100 and a method name.".into());
        }
        if request.action == "network.headers"
            && request
                .id
                .as_ref()
                .is_none_or(|s| s.is_empty() || s.len() > 128)
        {
            return Err(
                "Header reads need requestId from network.list for the current document.".into(),
            );
        }
        Ok(request)
    }
    pub(crate) fn name(&self) -> &'static str {
        if self.action == "network.headers" {
            "network.headers"
        } else {
            "network.list"
        }
    }
    fn result(&self, snapshot: &Value) -> Result<Value, String> {
        let rows = snapshot["requests"].as_array().ok_or(
            "The engine returned no request records; reload the page before reading requests.",
        )?;
        if self.action == "network.headers" {
            let row=rows.iter().find(|r|r["requestId"].as_str()==self.id.as_deref()).ok_or("This request expired or belongs to another document; use a current requestId from network.list.")?;
            return Ok(headers(row, self.unredacted));
        }
        let matching: Vec<_> = rows
            .iter()
            .filter(|r| {
                (!self.failed_only
                    || r.get("error").is_some()
                    || r["status"].as_u64().is_some_and(|s| s >= 400))
                    && self
                        .url
                        .as_ref()
                        .is_none_or(|s| clean_url(&r["url"]).contains(s))
                    && self.method.as_ref().is_none_or(|s| {
                        r["method"]
                            .as_str()
                            .is_some_and(|m| m.eq_ignore_ascii_case(s))
                    })
                    && self
                        .status
                        .is_none_or(|s| r["status"].as_u64() == Some(u64::from(s)))
            })
            .collect();
        let total = matching.len();
        let offset = self.last.map_or(self.offset, |n| total.saturating_sub(n));
        let mut entries = Vec::new();
        let mut bytes = 0;
        for row in matching
            .into_iter()
            .skip(offset)
            .take(self.last.or(self.limit).unwrap_or(25))
        {
            let mut item = json!({"requestId":row["requestId"],"url":clean_url(&row["url"]),"method":row["method"],"status":row["status"],"finished":row["finished"]});
            if let Some(error) = row["error"].as_str() {
                item["error"] = json!(error.chars().take(220).collect::<String>());
            }
            if bytes + item.to_string().len() > 1700 {
                break;
            }
            bytes += item.to_string().len();
            entries.push(item);
        }
        let next = offset + entries.len();
        Ok(
            json!({"requests":entries,"total":total,"offset":offset,"nextOffset":(next<total).then_some(next),"dropped":snapshot["dropped"],"coverage":snapshot["coverage"]}),
        )
    }
}
fn clean_url(value: &Value) -> String {
    let Ok(mut url) = url::Url::parse(value.as_str().unwrap_or_default()) else {
        return "[unavailable URL]".into();
    };
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_fragment(None);
    if url.query().is_some() {
        url.set_query(Some("[redacted]"));
    }
    url.as_str().chars().take(400).collect()
}
fn headers(row: &Value, raw: bool) -> Value {
    let mut out = json!({"requestId":row["requestId"],"unredacted":raw,"truncated":row["requestHeadersTruncated"]==true || row["responseHeadersTruncated"]==true});
    let mut bytes = 0;
    for (source, destination) in [
        ("requestHeaders", "request"),
        ("responseHeaders", "response"),
    ] {
        let mut headers = serde_json::Map::new();
        if let Some(map) = row[source].as_object() {
            for (name, value) in map {
                let safe = matches!(
                    name.to_ascii_lowercase().as_str(),
                    "content-type"
                        | "content-length"
                        | "content-encoding"
                        | "cache-control"
                        | "accept"
                        | "accept-encoding"
                );
                let value = if raw || safe {
                    value.as_str().unwrap_or_default()
                } else {
                    "[redacted]"
                };
                let size = name.len() + value.len() + 8;
                if bytes + size > 1700 {
                    out["truncated"] = json!(true);
                    continue;
                }
                bytes += size;
                headers.insert(name.clone(), json!(value));
            }
        }
        out[destination] = json!(headers);
    }
    out
}
impl Browser {
    pub(crate) async fn network(
        &self,
        session: &str,
        request: &Request,
        by: Actor,
    ) -> Result<Value, ActError> {
        let bound = self.bind_read(session, by).await.map_err(refused)?;
        if request.unredacted && self.service.shots_blocked(session) {
            return Err(refused(
                "Unredacted headers are blocked after a vault login because they could contain credentials; read redacted headers or open a fresh document.",
            ));
        }
        let mount = bound
            .read_mount
            .as_ref()
            .ok_or_else(|| refused(crate::NOTHING_OPEN))?;
        let snapshot = self
            .service
            .network_records(mount)
            .await
            .map_err(|e| failed(crate::driver::preview_text(&e)))?;
        bound.check_read_mount().map_err(refused)?;
        let data = request.result(&snapshot).map_err(refused)?;
        let text = data.to_string();
        let text = vw_secrets::scrub(text.as_bytes())
            .map_or(text, |(v, _)| String::from_utf8_lossy(&v).into_owned());
        let data: Value = serde_json::from_str(&text).map_err(|_| {
            failed("Request details could not be redacted; read the page before continuing.")
        })?;
        Ok(
            json!({"ok":true,"action":request.name(),"Outcome":"Native request records read.","untrustedPageContent":data}),
        )
    }
}
