//! Passive Safari exports use named, document-bound chunks and the shared chat-folder writer.
//! No caller code or arbitrary URL is accepted; packed resources come only from the snapshot's cache inventory.
use super::{owner, page_files, read_consent::Consent};
use crate::core::Shared;
use base64::Engine as _;
use serde_json::{Value, json};
use std::sync::Arc;
use vw_browser::capture::{Fetch, FetchFuture, Fetched, Format, media, page, single_html};

struct Snapshot<'a> {
    shared: &'a Arc<Shared>,
    chat: &'a str,
    request: &'a Value,
    consent: Option<&'a Consent>,
    cursor: String,
}
impl Snapshot<'_> {
    fn check(&self) -> Result<(), String> {
        super::check::authorize_read(
            self.shared,
            self.chat,
            "chrome_execute",
            self.request,
            self.consent,
        )?;
        super::gateway::validate_session(self.shared, self.chat, "chrome_execute", self.request)?;
        super::reconnect::stopped(
            self.shared,
            self.chat,
            super::agent_browser::Browser::Safari,
        )
    }
    async fn collect(&self, mut args: Value) -> Result<(Value, String), String> {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
        let mut held = None;
        let mut text = String::new();
        let mut received = 0;
        loop {
            self.check()?;
            if tokio::time::Instant::now() >= deadline {
                return Err(
                    "The Safari export transfer timed out. Start the capture again.".into(),
                );
            }
            let mut request = self.request.clone();
            request["args"] = args.clone();
            let answer = super::escalation::send(
                self.shared,
                self.chat,
                "chrome_execute",
                request,
                self.consent,
            )
            .await?;
            self.check()?;
            let body = &answer.value()["structuredContent"];
            if body["status"] != "completed" {
                return Err(format!(
                    "Safari did not complete this export: {}. Read the page before capturing again.",
                    body["error"]["message"]
                ));
            }
            let data = &body["data"];
            append(&mut text, &mut received, &mut held, data)?;
            if data["nextOffset"].is_null() {
                return Ok((
                    held.ok_or("The export metadata is missing. Start the capture again.")?,
                    text,
                ));
            }
            args["cursor"] = data["cursor"].clone();
            args["offset"] = data["nextOffset"].clone();
        }
    }
}
fn append(
    text: &mut String,
    received: &mut u64,
    held: &mut Option<Value>,
    data: &Value,
) -> Result<(), String> {
    let bad =
        "The Safari export chunks do not form one complete snapshot. Start the capture again.";
    let length = data["length"]
        .as_u64()
        .filter(|n| *n <= 10_000_000)
        .ok_or(bad)?;
    let chunk = data["chunk"]
        .as_str()
        .filter(|s| s.len() <= 768_000)
        .ok_or(bad)?;
    if data["cursor"]
        .as_str()
        .is_none_or(|s| s.is_empty() || s.len() > 128)
    {
        return Err(bad.into());
    }
    if let Some(first) = held {
        for field in [
            "cursor", "length", "format", "url", "encoding", "resource", "bytes",
        ] {
            if first[field] != data[field] {
                return Err(bad.into());
            }
        }
    } else {
        let mut meta = data.clone();
        meta.as_object_mut().ok_or(bad)?.remove("chunk");
        *held = Some(meta);
    }
    let next = *received + chunk.encode_utf16().count() as u64;
    if next > length
        || data["nextOffset"].is_null() && next != length
        || !data["nextOffset"].is_null()
            && (data["nextOffset"].as_u64() != Some(next) || next <= *received || next >= length)
        || text.len() + chunk.len() > page_files::MAX
    {
        return Err(bad.into());
    }
    text.push_str(chunk);
    *received = next;
    Ok(())
}
impl Fetch for Snapshot<'_> {
    fn fetch<'a>(&'a self, _session: &'a str, url: &'a str, max_bytes: u64) -> FetchFuture<'a> {
        Box::pin(async move {
            let mut args = self.request["args"].clone();
            args["cursor"] = json!(self.cursor);
            args["resource"] = json!(url);
            let (meta, encoded) = self.collect(args).await?;
            if meta["encoding"] != "base64" || meta["resource"] != url || encoded.len() > 7_000_000
            {
                return Err(
                    "Safari returned an invalid cached resource. Keep its link instead.".into(),
                );
            }
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .map_err(|_| "Safari returned invalid resource bytes. Keep its link instead.")?;
            if bytes.len() as u64 > max_bytes || meta["bytes"].as_u64() != Some(bytes.len() as u64)
            {
                return Err(
                    "The cached resource exceeds its verified size bound. Keep its link instead."
                        .into(),
                );
            }
            Ok(Fetched {
                bytes,
                content_type: meta["mime"].as_str().map(str::to_owned),
            })
        })
    }
}
pub(super) async fn send(
    shared: &Arc<Shared>,
    chat: &str,
    request: &Value,
    consent: Option<&Consent>,
) -> Result<owner::Answer, String> {
    if request["args"].as_object().is_none_or(|a| {
        a.keys()
            .any(|key| !(key == "tabId" || request["action"] == "capture.page" && key == "format"))
    }) {
        return Err("Unknown capture argument. Use only tabId and the advertised format.".into());
    }
    let _ = super::output_file::folder(shared, chat)?;
    let mut snapshot = Snapshot {
        shared,
        chat,
        request,
        consent,
        cursor: String::new(),
    };
    let (meta, text) = snapshot.collect(request["args"].clone()).await?;
    snapshot.cursor = meta["cursor"].as_str().unwrap_or_default().to_owned();
    if request["action"] == "capture.media" {
        return save_media(&snapshot, &text).await;
    }
    let format: Format = serde_json::from_value(request["args"]["format"].clone())
        .map_err(|_| "Capture needs markdown, text, html or single_html. Choose one format.")?;
    let (text, linked) = if format == Format::SingleHtml {
        if meta["cut"] == true {
            return Err("The packed page was truncated. Save HTML or text instead.".into());
        }
        let document: single_html::Document = serde_json::from_str(&text)
            .map_err(|_| "The packed document is invalid. Capture HTML instead.")?;
        single_html::pack(document, "", &snapshot, 16 * 1024 * 1024).await?
    } else {
        (text, 0)
    };
    snapshot.check()?;
    let (suffix, mime) = match format {
        Format::Markdown => ("md", "text/markdown"),
        Format::Text => ("txt", "text/plain"),
        _ => ("html", "text/html"),
    };
    let text = page::file_text(
        format,
        &text,
        meta["url"].as_str().unwrap_or_default(),
        &chrono::Utc::now().to_rfc3339(),
        meta["cut"] == true,
    );
    let file = page_files::save(shared, chat, suffix, mime, text.as_bytes())?;
    let unpacked = linked
        + usize::try_from(meta["unavailableStyles"].as_u64().unwrap_or(0))
            .map_err(|_| "The export resource count is invalid. Capture again.")?;
    Ok(super::page_plan::mcp(
        &json!({"status":"completed","action":"capture.page","capability":"page.read","data":{"file":file,"url":meta["url"],"title":meta["title"],"truncated":meta["cut"]==true,"unpackedResources":unpacked,"coverage":meta["coverage"],"resourceCoverage":"Only loaded same-origin HTTP-cache assets; cached bytes can differ from the decoded page image. No new resource requests. Cross-origin or uncached assets remain links.","instruction":if unpacked>0 {"Some resources were unavailable and remain links. Inspect unpackedResources before relying on an offline copy."}else{"The passive page export is saved in the chat folder. Open the returned file to inspect it."}}}),
    ))
}
async fn save_media(snapshot: &Snapshot<'_>, text: &str) -> Result<owner::Answer, String> {
    let listing: media::Listing = serde_json::from_str(text)
        .map_err(|_| "The Safari media inventory is invalid. Capture the page again.")?;
    let mut total = 0;
    let mut entries = Vec::new();
    let deadline = tokio::time::Instant::now() + media::STEP_LIMIT;
    for item in listing.items.into_iter().take(40) {
        snapshot.check()?;
        let mut entry = json!({"kind":item.kind,"url":item.url,"status":"notLoaded"});
        let bytes = if let Some(svg) = item.svg {
            Ok(svg.into_bytes())
        } else if item.loaded {
            tokio::time::timeout_at(
                deadline,
                snapshot.fetch("", item.url.as_deref().unwrap_or_default(), media::MAX_FILE),
            )
            .await
            .unwrap_or_else(|_| {
                Err("The media capture time limit was reached. Capture fewer resources.".into())
            })
            .map(|value| value.bytes)
        } else {
            entries.push(entry);
            continue;
        };
        snapshot.check()?;
        match bytes.and_then(|bytes| {
            super::page_capture::save_resource(snapshot.shared, snapshot.chat, &bytes, &mut total)
        }) {
            Ok(file) => {
                entry["status"] = json!("saved");
                entry["file"] = file;
            }
            Err(reason) => {
                entry["status"] = json!("notSaved");
                entry["reason"] = json!(reason);
            }
        }
        entries.push(entry);
    }
    snapshot.check()?;
    let manifest = json!({"url":listing.url,"title":listing.title,"items":entries});
    let file = page_files::save(
        snapshot.shared,
        snapshot.chat,
        "json",
        "application/json",
        manifest.to_string().as_bytes(),
    )?;
    Ok(super::page_plan::mcp(
        &json!({"status":"completed","action":"capture.media","capability":"page.read","data":{"manifest":file,"items":entries,"resourceCoverage":"Loaded same-origin HTTP-cache images/fonts and sanitized inline SVG. Cached bytes may differ from the decoded page image; unavailable originals are reported individually.","instruction":"The manifest lists each saved or unavailable resource. Inspect it before relying on a complete media export."}}),
    ))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chunks_reject_identity_changes_gaps_and_early_completion() {
        let first = json!({"cursor":"c","format":"text","url":"https://a.test","length":6,"chunk":"abc","nextOffset":3});
        for second in [
            json!({"cursor":"other","format":"text","url":"https://a.test","length":6,"chunk":"def","nextOffset":null}),
            json!({"cursor":"c","format":"text","url":"https://b.test","length":6,"chunk":"def","nextOffset":null}),
            json!({"cursor":"c","format":"text","url":"https://a.test","length":6,"chunk":"d","nextOffset":null}),
        ] {
            let (mut text, mut count, mut held) = (String::new(), 0, None);
            append(&mut text, &mut count, &mut held, &first).unwrap();
            assert!(append(&mut text, &mut count, &mut held, &second).is_err());
        }
    }
}
