//! Reads only inventoried image/font resource bodies from the same live document, via frozen CDP primitives.
//! Page capture supplies the fixed request and isolated document token; no caller URL or raw code is accepted.
use super::{page_files, page_plan, read_consent::Consent};
use crate::core::Shared;
use base64::Engine as _;
use serde_json::{Value, json};
use std::sync::Arc;
use vw_browser::capture::{Fetch, FetchFuture, Fetched};

pub(super) struct Resources<'a> {
    pub shared: &'a Arc<Shared>,
    pub chat: &'a str,
    pub request: &'a Value,
    pub consent: Option<&'a Consent>,
    pub key: &'a str,
}
pub(super) fn eval(id: &str, expression: &str) -> Value {
    json!({"id":id,"op":"cdp","method":"Runtime.evaluate","world":"agent","params":{"expression":expression,"returnByValue":true}})
}
impl Resources<'_> {
    pub async fn script(&self, expression: &str) -> Result<(Value, Value), String> {
        let result = page_plan::dispatch(
            self.shared,
            self.chat,
            self.request,
            self.consent,
            json!([eval("page", expression)]),
        )
        .await?;
        let rows = page_files::results(self.shared, self.chat, &result)?;
        let value = rows
            .pointer("/page/result/value")
            .and_then(Value::as_str)
            .and_then(|s| serde_json::from_str::<Value>(s).ok())
            .ok_or("The page capture could not be read; reload the page and capture again.")?;
        if value.get("error").is_some() {
            return Err(format!(
                "Page capture failed: {}. Read the page and capture again.",
                value["error"]
            ));
        }
        Ok((result, value))
    }
    pub fn guard(&self) -> String {
        format!("Boolean(window[{}])", json!(self.key))
    }
}
impl Fetch for Resources<'_> {
    fn fetch<'a>(&'a self, _session: &'a str, url: &'a str, max_bytes: u64) -> FetchFuture<'a> {
        Box::pin(async move {
            let result = page_plan::dispatch(self.shared,self.chat,self.request,self.consent,json!([
                eval("before",&self.guard()),
                {"id":"tree","op":"cdp","method":"Page.getFrameTree"},
                {"id":"asset","op":"cdp","method":"Page.getResourceContent","params":{"frameId":{"$result":"tree#/frameTree/frame/id"},"url":url}},
                eval("after",&self.guard())
            ])).await?;
            let rows = page_files::results(self.shared, self.chat, &result)?;
            if rows.pointer("/before/result/value") != Some(&json!(true))
                || rows.pointer("/after/result/value") != Some(&json!(true))
            {
                return Err(
                    "The document changed during capture; capture its current page again.".into(),
                );
            }
            let body = rows["asset"]["content"].as_str().ok_or(
                "The browser no longer holds this resource; reload the page and capture again.",
            )?;
            let bytes = if rows["asset"]["base64Encoded"] == true {
                base64::engine::general_purpose::STANDARD
                    .decode(body)
                    .map_err(|_| "The browser returned invalid resource bytes; capture again.")?
            } else {
                body.as_bytes().to_vec()
            };
            if bytes.len() as u64 > max_bytes {
                return Err(
                    "This resource exceeds the capture size limit; choose a smaller resource."
                        .into(),
                );
            }
            Ok(Fetched {
                bytes,
                content_type: None,
            })
        })
    }
}
