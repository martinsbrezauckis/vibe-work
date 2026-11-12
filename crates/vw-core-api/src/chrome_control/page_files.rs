//! Bounded primitive transfer receipts and page exports; only authenticated chat-folder files are opened.
//! The fixed page compiler calls this after receipt completion; page text never selects a destination.
use crate::core::Shared;
use serde_json::{Value, json};
use std::{io::Read, path::Path};
pub(super) const MAX: usize = 32 * 1024 * 1024;
pub(super) fn token() -> Result<String, String> {
    let mut random = [0u8; 16];
    getrandom::fill(&mut random)
        .map_err(|_| "A capture identifier could not be allocated; try again.")?;
    Ok(format!("{:032x}", u128::from_be_bytes(random)))
}

fn transferred(shared: &Shared, chat: &str, receipt: &Value) -> Result<Value, String> {
    let fail = "The transferred page result is unavailable or outside this chat folder; capture the page again.";
    let folders = crate::agent_tools::session::folders(shared, chat)?;
    let folder = folders.first().ok_or(fail)?;
    let filename = receipt["filename"].as_str().ok_or(fail)?;
    let relative = Path::new(filename).strip_prefix(folder).map_err(|_| fail)?;
    let size = receipt["size"].as_u64().ok_or(fail)?;
    if receipt["inSessionFolder"] != true || size > MAX as u64 {
        return Err(fail.into());
    }
    let file = crate::search_walk::open(Path::new(folder), relative.to_str().ok_or(fail)?)
        .map_err(|_| fail)?;
    if file.metadata().map_err(|_| fail)?.len() != size {
        return Err(fail.into());
    }
    let mut bytes = Vec::new();
    file.take(MAX as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| fail)?;
    if bytes.len() as u64 != size {
        return Err(fail.into());
    }
    serde_json::from_slice(&bytes).map_err(|_| fail.into())
}

pub(super) fn results(shared: &Shared, chat: &str, body: &Value) -> Result<Value, String> {
    if body["status"] != "completed" {
        return Err(format!(
            "Page capture failed: {}. Read browser state before trying again.",
            body["error"]
        ));
    }
    let mut rows = if body["data"]["resultsFile"].is_object() {
        transferred(shared, chat, &body["data"]["resultsFile"])?
    } else {
        body["data"]["results"].clone()
    };
    let values = rows
        .as_object_mut()
        .ok_or("Page capture returned no results; capture again.")?;
    for value in values.values_mut() {
        if let Some(handle) = value["handle"].as_str() {
            *value = transferred(shared, chat, &body["data"]["outputs"][handle])?;
        }
    }
    Ok(rows)
}

pub(crate) fn save(
    shared: &Shared,
    chat: &str,
    suffix: &str,
    mime: &str,
    bytes: &[u8],
) -> Result<Value, String> {
    if bytes.len() > MAX {
        return Err(
            "The capture exceeds 32 MiB; capture a smaller page or fewer resources.".into(),
        );
    }
    let folder = super::output_file::folder(shared, chat)?;
    let clean = vw_secrets::scrub(bytes);
    let bytes = clean.as_ref().map_or(bytes, |(b, _)| b.as_slice());
    let name = format!("browser-{}.{suffix}", token()?);
    super::output_file::write(&folder, &name, bytes).map_err(
        |_| "The capture could not be saved in the chat folder; choose a writable folder.",
    )?;
    if folder
        .metadata(&name)
        .map_err(|_| "The saved capture could not be read back; inspect the chat folder.")?
        .len()
        != bytes.len() as u64
    {
        return Err(
            "The saved capture size did not match; inspect the chat folder before retrying.".into(),
        );
    }
    Ok(json!({"path":name,"bytes":bytes.len(),"mime":mime}))
}
