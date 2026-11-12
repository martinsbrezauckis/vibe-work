//! Bounded console and page-error reads/clears over the preview's per-document buffers.
//! Called by the browser action loop; all page words stay untrusted and stored secrets are scrubbed.
use crate::{
    ActError, Actor, Browser,
    driver::{failed, refused},
};
use serde::Deserialize;
use serde_json::{Value, json};

/// Checked console or error request, with filters applied before paging.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    action: String,
    #[serde(default)]
    offset: usize,
    limit: Option<usize>,
    last: Option<usize>,
    level: Option<String>,
    text: Option<String>,
}

impl Request {
    pub(crate) fn parse(args: &Value) -> Result<Self, String> {
        let value: Self = serde_json::from_value(args.clone())
            .map_err(|_| "Diagnostics need integer offset/limit and string level/text filters.")?;
        if value.offset > 20_000
            || value.limit.is_some_and(|v| !(1..=100).contains(&v))
            || value.last.is_some_and(|v| !(1..=100).contains(&v))
            || value.last.is_some() && args.get("offset").is_some()
            || value.text.as_ref().is_some_and(|s| s.len() > 2000)
            || value.level.as_deref().is_some_and(|s| {
                ![
                    "log",
                    "info",
                    "debug",
                    "warn",
                    "error",
                    "exception",
                    "rejection",
                    "resource",
                ]
                .contains(&s)
            })
        {
            return Err("Diagnostics accept limit/last 1–100, offset 0–20000, and a known level or text up to 2000 bytes; last excludes offset.".into());
        }
        if value.clear()
            && ["offset", "limit", "last", "level", "text"]
                .iter()
                .any(|k| args.get(*k).is_some())
        {
            return Err("Clear takes no filters; use console.clear or page.errors.clear for the selected buffer.".into());
        }
        Ok(value)
    }

    pub(crate) fn name(&self) -> &'static str {
        match self.action.as_str() {
            "console.clear" => "console.clear",
            "page.errors.clear" => "page.errors.clear",
            "page.errors" => "page.errors",
            _ => "console.read",
        }
    }

    fn clear(&self) -> bool {
        matches!(self.action.as_str(), "console.clear" | "page.errors.clear")
    }

    fn page(&self, records: Vec<vw_preview::ConsoleEntry>, cleared: usize, dropped: u64) -> Value {
        let records: Vec<_> = records
            .into_iter()
            .map(|r| json!(r))
            .filter(|r| {
                self.level.as_ref().is_none_or(|level| r["level"] == *level)
                    && self
                        .text
                        .as_ref()
                        .is_none_or(|text| r["text"].as_str().is_some_and(|s| s.contains(text)))
            })
            .collect();
        let total = records.len();
        let offset = self.last.map_or(self.offset, |n| total.saturating_sub(n));
        let limit = self.last.or(self.limit).unwrap_or(25);
        let mut entries = Vec::new();
        let mut bytes = 0;
        for mut row in records.into_iter().skip(offset).take(limit) {
            while row.to_string().len() > 1500 {
                for field in ["text", "url"] {
                    if let Some(text) = row[field].as_str() {
                        row[field] = json!(
                            text.chars()
                                .take(text.chars().count().div_ceil(2))
                                .collect::<String>()
                        );
                    }
                }
                row["truncated"] = json!(true);
            }
            let size = row.to_string().len();
            if bytes + size > 1900 {
                break;
            }
            bytes += size;
            entries.push(row);
        }
        let next = offset.saturating_add(entries.len());
        json!({"entries":entries,"total":total,"offset":offset,"nextOffset":(next < total).then_some(next),"cleared":cleared,"dropped":dropped})
    }
}

impl Browser {
    pub(crate) async fn diagnostics(
        &self,
        session: &str,
        request: &Request,
        by: Actor,
    ) -> Result<Value, ActError> {
        let bound = self.bind_read(session, by).await.map_err(refused)?;
        let page = bound.page_of(session).await.map_err(refused)?;
        let url = self
            .service
            .view(session)
            .and_then(|v| v.url)
            .ok_or_else(|| refused(crate::NOTHING_OPEN))?;
        bound.check_answer(session, &page, &url).map_err(refused)?;
        let (records, cleared, dropped) = self
            .service
            .diagnostic_records(
                session,
                request.action.starts_with("page."),
                request.clear(),
            )
            .await
            .map_err(|e| failed(crate::driver::preview_text(&e)))?;
        bound.check_read_mount().map_err(refused)?;
        let data = request.page(records, cleared, dropped);
        let text = data.to_string();
        let text = vw_secrets::scrub(text.as_bytes())
            .map_or(text, |(v, _)| String::from_utf8_lossy(&v).into_owned());
        let data: Value = serde_json::from_str(&text)
            .map_err(|_| failed("Diagnostics could not be redacted; read the page again."))?;
        Ok(
            json!({"Outcome":if request.clear() {"Diagnostic buffer cleared."} else {"Diagnostic buffer read."},"ok":true,"action":request.name(),"untrustedPageContent":data}),
        )
    }
}
