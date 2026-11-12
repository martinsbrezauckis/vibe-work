//! Rich clipboard browser actions through the serialized native owner; shared Full policy runs before this route.
//! Converts bounded HTML/PNG readback to untrusted text or a tool image and keeps vault copies withheld.
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use vw_platform::{
    clipboard::{ClipboardError, ClipboardService},
    clipboard_content::{ClipboardContent, ClipboardFormat},
};
pub(super) async fn run(clipboard: &ClipboardService, args: &Value) -> Result<Value, String> {
    let format =
        match args["mime"].as_str() {
            Some("text/html") => ClipboardFormat::Html,
            Some("image/png") => ClipboardFormat::Png,
            _ => return Err(
                "Rich clipboard needs mime text/html or image/png. Choose one supported format."
                    .into(),
            ),
        };
    let write = args["action"] == "clipboard.write_rich";
    let mut expected = None;
    let actual = if write {
        let content = match format {
            ClipboardFormat::Html => {
                if args.get("base64").is_some() {
                    return Err(
                        "HTML clipboard writes take text. Omit base64 and supply HTML text.".into(),
                    );
                }
                let text=args["text"].as_str().filter(|v|v.len()<=60_000).ok_or("HTML clipboard writes need text up to 60000 bytes. Supply a smaller HTML fragment.")?;
                ClipboardContent::Html(text.into())
            }
            ClipboardFormat::Png => {
                if args.get("text").is_some() {
                    return Err(
                        "PNG clipboard writes take base64. Omit text and supply PNG bytes.".into(),
                    );
                }
                let encoded=args["base64"].as_str().filter(|v|v.len()<=349_528).ok_or("PNG clipboard writes need base64 up to 256 KiB decoded. Supply a smaller PNG image.")?;
                ClipboardContent::Png(
                    STANDARD
                        .decode(encoded)
                        .map_err(|_| "PNG base64 is invalid. Supply encoded PNG bytes.")?,
                )
            }
        };
        expected = Some(content.clone());
        clipboard
            .write_content(content)
            .await
            .map_err(|error| reason(&error))?
    } else {
        clipboard
            .read_content(format)
            .await
            .map_err(|error| reason(&error))?
    };
    let mut result = json!({"Outcome":if write{"Clipboard content written and read back."}else{"Clipboard content read."},"mime":args["mime"],"readBack":true});
    if write {
        result["written"] = json!(true);
        result["matchesRequestedBytes"] = json!(expected.as_ref() == Some(&actual));
        if expected.as_ref() != Some(&actual) {
            result["Outcome"] = json!(
                "Clipboard content differs from the requested bytes. Inspect the returned content before continuing."
            );
        }
    }
    match actual {
        ClipboardContent::Html(html) => {
            let html = vw_secrets::scrub(html.as_bytes()).map_or(html, |(bytes, _)| {
                String::from_utf8_lossy(&bytes).into_owned()
            });
            let mut end = html.len().min(65_536);
            while !html.is_char_boundary(end) {
                end -= 1;
            }
            result["untrustedPageContent"] =
                json!({"html":&html[..end],"bytes":html.len(),"truncated":end<html.len()});
        }
        ClipboardContent::Png(bytes) => {
            result["bytes"] = json!(bytes.len());
            result[vw_agent_tools::wire::IMAGE_KEY] =
                json!({"data":STANDARD.encode(bytes),"mimeType":"image/png"});
        }
    }
    Ok(result)
}
fn reason(error: &ClipboardError) -> String {
    match error{
        ClipboardError::Unsupported(reason)=>(*reason).into(),
        _=>"Clipboard content is unavailable, invalid, too large, or withheld during a vault copy; a write may already have occurred. Inspect the system clipboard before choosing another action.".into()
    }
}
