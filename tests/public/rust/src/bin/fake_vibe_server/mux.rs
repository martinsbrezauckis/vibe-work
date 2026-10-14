//! `--multiplex` mode of the fake server: Vibe's channel framing (`app_server/multiplex.py`, 2.26.0).
//!
//! Rules copied from Vibe: a frame for an unknown id opens that channel only when the id is above every
//! id opened before (a lower one is ignored, like Vibe's watermark); `message: null` closes the channel
//! (its session's work ends, then the close is acknowledged with `{"channelId", "message": null}`);
//! stdin EOF closes every open channel; `session/stop` (script mode) is answered, then its channel is
//! closed by the server like Vibe's channel server closing itself. Deviation on purpose: an invalid outer frame exits the process
//! with code 4 and a stderr line (Vibe's strict validation also ends the whole process), so a client
//! that ever writes a bad frame fails its test loudly.

use std::collections::BTreeMap;
use std::io::BufRead;
use std::sync::Arc;

use serde_json::{Value, json};

use crate::script::Send;
use crate::{Conn, Ctx};

fn bad_frame(why: &str) -> ! {
    eprintln!("fake multiplex: invalid channel frame ({why}); exiting like Vibe");
    std::process::exit(4);
}

/// The channel id of an outer frame, checked like Vibe (`^[1-9][0-9]{0,15}$`, a string).
fn channel_id(frame: &Value) -> u64 {
    let Some(object) = frame.as_object() else {
        bad_frame("not an object")
    };
    if object.keys().any(|k| k != "channelId" && k != "message") {
        bad_frame("extra key");
    }
    let Some(text) = object.get("channelId").and_then(Value::as_str) else {
        bad_frame("channelId missing or not a string")
    };
    let valid = (1..=16).contains(&text.len())
        && !text.starts_with('0')
        && text.bytes().all(|b| b.is_ascii_digit());
    if !valid {
        bad_frame("channelId pattern");
    }
    match object.get("message") {
        Some(Value::Object(_) | Value::Null) => {}
        _ => bad_frame("message is neither an object nor null"),
    }
    text.parse().unwrap_or_else(|_| bad_frame("channelId"))
}

/// Serves channels until stdin ends.
pub fn serve(ctx: &Ctx, raw: &Send) {
    let mut channels: BTreeMap<u64, Conn> = BTreeMap::new();
    let mut watermark = 0_u64;
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(frame) = serde_json::from_str::<Value>(&line) else {
            bad_frame("not JSON");
        };
        let id = channel_id(&frame);
        let message = &frame["message"];
        if let std::collections::btree_map::Entry::Vacant(slot) = channels.entry(id) {
            if id <= watermark || message.is_null() {
                continue;
            }
            watermark = id;
            let raw = raw.clone();
            let send: Send = Arc::new(move |inner: Value| {
                raw(json!({"channelId": id.to_string(), "message": inner}));
            });
            if let Some(script) = &ctx.scripted {
                script.connect(id, send.clone());
            }
            slot.insert(Conn { key: id, send });
        }
        if message.is_null() {
            close(ctx, raw, &mut channels, id);
            continue;
        }
        let stop = channels
            .get(&id)
            .is_some_and(|conn| ctx.on_frame(conn, message));
        if stop {
            close(ctx, raw, &mut channels, id);
        }
    }
    let open: Vec<u64> = channels.keys().copied().collect();
    for id in open {
        close(ctx, raw, &mut channels, id);
    }
}

fn close(ctx: &Ctx, raw: &Send, channels: &mut BTreeMap<u64, Conn>, id: u64) {
    if channels.remove(&id).is_none() {
        return;
    }
    if let Some(script) = &ctx.scripted {
        script.disconnect(id);
    }
    raw(json!({"channelId": id.to_string(), "message": null}));
}
