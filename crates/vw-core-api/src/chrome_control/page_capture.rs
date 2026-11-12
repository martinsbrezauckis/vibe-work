//! Named passive page and loaded-media exports through the frozen transport; shares built-in sanitizers.
//! Called by the checked page compiler; bounded files and manifests are saved inside the chat folder.
use super::{owner, page_files, page_resources::Resources, read_consent::Consent};
use crate::core::Shared;
use serde_json::{Value, json};
use std::sync::Arc;
use vw_browser::capture::{Fetch as _, Format, media, page, single_html};
pub(super) const NAMES: &[&str] = &["capture.page", "capture.media"];
const MAX_CHARS: usize = 1_000_000;

pub(super) fn schema(action: &str) -> Value {
    let mut properties = json!({"tabId":{"type":"integer","minimum":0,"maximum":2_147_483_647}});
    let mut required = vec!["tabId"];
    if action == "capture.page" {
        properties["format"] =
            json!({"type":"string","enum":["markdown","text","html","single_html"]});
        required.push("format");
    }
    json!({"type":"object","additionalProperties":false,"properties":properties,"required":required})
}

pub(super) async fn send(
    shared: &Arc<Shared>,
    chat: &str,
    request: &Value,
    consent: Option<&Consent>,
) -> Result<owner::Answer, String> {
    #[cfg(feature = "your-safari")]
    if super::agent_browser::selected(shared, chat, "chrome_execute", request)
        == super::agent_browser::Browser::Safari
    {
        return super::safari_capture::send(shared, chat, request, consent).await;
    }
    if request["args"]
        .as_object()
        .is_none_or(|a| a.keys().any(|k| !["tabId", "format"].contains(&k.as_str())))
    {
        return Err("Unknown capture argument; use the advertised capture schema.".into());
    }
    let _ = super::output_file::folder(shared, chat)?;
    let key = format!("__vw_capture_{}", page_files::token()?);
    let resource = Resources {
        shared,
        chat,
        request,
        consent,
        key: &key,
    };
    let result = if request["action"] == "capture.media" {
        save_media(&resource).await
    } else {
        save_page(&resource).await
    };
    // Best-effort cleanup cannot turn an uncertain or failed export into success.
    let _ = resource
        .script(&format!(
            "(()=>{{delete window[{}];return '{{}}';}})()",
            json!(key)
        ))
        .await;
    result.map(|body| super::page_plan::mcp(&body))
}

async fn save_page(resource: &Resources<'_>) -> Result<Value, String> {
    let format: Format = serde_json::from_value(resource.request["args"]["format"].clone())
        .map_err(|_| "Capture needs format markdown, text, html or single_html; choose one.")?;
    let script = vw_browser::scripts::page_text(
        resource.key,
        &json!({"op":"start","format":resource.request["args"]["format"],"token":resource.key,"maxChars":MAX_CHARS,"chunk":MAX_CHARS}),
    );
    let expression = format!(
        "(()=>{{window[{}]={{}};return ({script});}})()",
        json!(resource.key)
    );
    let (mut result, data) = resource.script(&expression).await?;
    let text = data["chunk"]
        .as_str()
        .ok_or("The page returned no export text; read the page and capture again.")?;
    let cut = data["cut"] == true;
    let (text, linked) = if format == Format::SingleHtml {
        if cut {
            return Err("This page exceeds the one-million-character packed capture limit; save HTML or text instead.".into());
        }
        let document: single_html::Document = serde_json::from_str(text)
            .map_err(|_| "The packed page could not be read; save HTML instead.")?;
        single_html::pack(document, "", resource, 16 * 1024 * 1024).await?
    } else {
        (text.to_owned(), 0)
    };
    let ending =
        vw_browser::scripts::page_text(resource.key, &json!({"op":"end","token":resource.key}));
    resource.script(&ending).await?;
    let (suffix, mime) = match format {
        Format::Markdown => ("md", "text/markdown"),
        Format::Text => ("txt", "text/plain"),
        _ => ("html", "text/html"),
    };
    let text = page::file_text(
        format,
        &text,
        data["url"].as_str().unwrap_or_default(),
        &chrono::Utc::now().to_rfc3339(),
        cut,
    );
    let saved = page_files::save(
        resource.shared,
        resource.chat,
        suffix,
        mime,
        text.as_bytes(),
    )?;
    result["data"] = json!({"file":saved,"url":data["url"],"title":data["title"],"truncated":cut,"unpackedResources":linked,"instruction":if linked>0 {"Some resources were unavailable or over the size limit and remain links; inspect unpackedResources before relying on an offline copy."}else{"The page export is saved in the chat folder; open the returned file to inspect it."}});
    Ok(result)
}

async fn save_media(resource: &Resources<'_>) -> Result<Value, String> {
    let script = vw_browser::scripts::media(resource.key, 40);
    let expression = format!(
        "(()=>{{window[{}]={{}};return ({script});}})()",
        json!(resource.key)
    );
    let (mut result, data) = resource.script(&expression).await?;
    let listing: media::Listing = serde_json::from_value(data)
        .map_err(|_| "The page media listing could not be read; capture again.")?;
    let mut entries = Vec::new();
    let mut total = 0;
    let deadline = tokio::time::Instant::now() + media::STEP_LIMIT;
    for item in listing.items.into_iter().take(40) {
        let mut entry = json!({"kind":item.kind,"url":item.url,"status":"notLoaded"});
        let bytes = if let Some(svg) = item.svg {
            Ok(svg.into_bytes())
        } else if item.loaded {
            tokio::time::timeout_at(
                deadline,
                resource.fetch("", item.url.as_deref().unwrap_or_default(), media::MAX_FILE),
            )
            .await
            .unwrap_or_else(|_| {
                Err("The media capture time limit was reached; capture fewer resources.".into())
            })
            .map(|f| f.bytes)
        } else {
            entries.push(entry);
            continue;
        };
        let saved = match bytes {
            Ok(bytes) => save_resource(resource.shared, resource.chat, &bytes, &mut total),
            Err(reason) => Err(reason),
        };
        match saved {
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
    let manifest = json!({"url":listing.url,"title":listing.title,"items":entries});
    let file = page_files::save(
        resource.shared,
        resource.chat,
        "json",
        "application/json",
        manifest.to_string().as_bytes(),
    )?;
    result["data"] = json!({"manifest":file,"items":entries,"instruction":"Saved resources and their individual outcomes are listed; use the manifest to see any unavailable items."});
    Ok(result)
}
pub(super) fn save_resource(
    shared: &Shared,
    chat: &str,
    bytes: &[u8],
    total: &mut usize,
) -> Result<Value, String> {
    let (ext,mime) = media::sniff(bytes).ok_or("This resource is not a supported image or font; use the page download action for other files.")?;
    let safe;
    let bytes = if ext == "svg" {
        safe = std::str::from_utf8(bytes)
            .ok()
            .and_then(vw_browser::capture::svg::sanitize)
            .ok_or("This SVG contains unsupported markup; save a screenshot instead.")?;
        safe.as_bytes()
    } else {
        bytes
    };
    if bytes.len() as u64 > media::MAX_FILE || *total + bytes.len() > 16 * 1024 * 1024 {
        return Err(
            "The resource exceeds the 5 MiB file or 16 MiB capture limit; capture fewer resources."
                .into(),
        );
    }
    let saved = page_files::save(shared, chat, ext, mime, bytes)?;
    *total += bytes.len();
    Ok(saved)
}
