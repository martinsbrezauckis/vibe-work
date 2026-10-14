//! Persists validated inline pictures in the project before forwarding a composer turn.
//! Depends on the gate's validation and `document_save`; direct, steer and queue sends share it.
//! Bytes remain inline for vision; an additional bounded resource names the readable absolute path.

use base64::Engine as _;
use serde_json::{Value, json};

use crate::core::Shared;
use crate::error::{CoreError, ErrorCode};

/// Adds readable project paths to already gate-validated image blocks.
pub(crate) async fn prepare(
    shared: &Shared,
    method: &str,
    params: &mut Value,
) -> Result<(), CoreError> {
    if !matches!(
        method,
        "turn/start" | "turn/steer" | "session/turn/enqueue" | "session/turn/queue/replace"
    ) {
        return Ok(());
    }
    let Some(id) = params["sessionId"].as_str().map(str::to_owned) else {
        return Ok(());
    };
    let cwd = shared.names.cwd_of(&shared.store, &id);
    let queued = method.starts_with("session/");
    let mut lists: Vec<&mut Vec<Value>> = if queued {
        params
            .get_mut("entries")
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
            .filter_map(|entry| entry.get_mut("content").and_then(Value::as_array_mut))
            .collect()
    } else {
        params
            .get_mut("message")
            .and_then(Value::as_array_mut)
            .into_iter()
            .collect()
    };
    for blocks in &mut lists {
        let mut notes = Vec::new();
        for block in blocks.iter().filter(|b| b["type"] == "image") {
            let (name, data) = if queued {
                (
                    block["altText"].as_str().unwrap_or("image"),
                    block["uri"]
                        .as_str()
                        .and_then(|s| s.split_once(',').map(|(_, data)| data)),
                )
            } else {
                (
                    block["attachment"]["alias"].as_str().unwrap_or("image"),
                    block["attachment"]["source"]["data"].as_str(),
                )
            };
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(data.unwrap_or_default())
                .map_err(|_| {
                    CoreError::new(
                        ErrorCode::InvalidParams,
                        "attachment image has invalid bytes",
                    )
                })?;
            let cwd = cwd.as_ref().ok_or_else(|| {
                CoreError::new(
                    ErrorCode::InvalidParams,
                    "Choose a project folder before attaching images.",
                )
            })?;
            let mut hash = blake3::Hasher::new();
            hash.update(name.as_bytes());
            hash.update(&[0]);
            hash.update(&bytes);
            let key = format!("img-{}", &hash.finalize().to_hex()[..32]);
            let folder = std::path::PathBuf::from(cwd);
            let sid = id.clone();
            let label = name.to_owned();
            let saved_key = key.clone();
            let path = tokio::task::spawn_blocking(move || {
                crate::document_save::save_image(&folder, &sid, &saved_key, &label, &bytes)
            })
            .await
            .map_err(|_| CoreError::new(ErrorCode::Internal, "attachment save task failed"))?
            .map_err(|reason| {
                CoreError::new(
                    ErrorCode::InvalidParams,
                    format!("Image could not be saved: {reason}"),
                )
            })?;
            let text = format!("Attached image {name:?}. Absolute path: {}", path.display());
            let uri = format!("vw://attachment/{key}");
            notes.push(if queued {
                json!({"type":"embedded_resource", "uri":uri, "text":text})
            } else {
                json!({"type":"resource", "resource":{"kind":"text", "uri":uri, "text":text}})
            });
        }
        blocks.extend(notes);
    }
    Ok(())
}
