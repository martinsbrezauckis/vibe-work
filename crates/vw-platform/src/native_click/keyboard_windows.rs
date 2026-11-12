//! `WebView2` keyboard delivery through its own protocol channel after native visibility and ownership checks.
//! Called by checked frame dispatch; key release completes before the host reads the event receipt.
use super::Keyboard;
use crate::native_click::{ClickPermission, ClickReply};
use crate::webview::devtools_win::DevTools;
use std::sync::atomic::{AtomicBool, Ordering};
use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Controller;

/// Sends one bounded keyboard operation to this native view.
/// # Errors
/// Refuses unavailable or unfocused native views before any input.
pub fn send(
    controller: &ICoreWebView2Controller,
    point: (f64, f64),
    input: Keyboard,
    sent: &AtomicBool,
    allowed: &ClickPermission,
    deadline: std::time::Instant,
    reply: ClickReply,
) {
    let channel = match super::super::windows_click::target(controller, point)
        .and_then(|_| DevTools::new(controller))
    {
        Ok(channel) => channel,
        Err(reason) => {
            let _ = reply.send(Err(reason));
            return;
        }
    };
    if reply.is_closed()
        || std::time::Instant::now() > deadline
        || allowed.as_ref().is_some_and(|check| !check())
    {
        let _ = reply.send(Err(
            "Not run: native keyboard admission changed. Read the page and try again.".into(),
        ));
        return;
    }
    match input {
        Keyboard::Text(text) => {
            sent.store(true, Ordering::Release);
            channel.call(
                "",
                "Input.insertText",
                &serde_json::json!({"text":text}).to_string(),
                Box::new(move |result| {
                    let _ = reply.send(result.map(|_| ()));
                }),
            );
        }
        Keyboard::Key(name, bits) => {
            let key = if name == "Space" { " " } else { &name };
            let printable = key.chars().count() == 1 && bits.trailing_zeros() >= 3;
            let text = if printable {
                key.to_owned()
            } else if key == "Enter" {
                "\r".into()
            } else {
                String::new()
            };
            let modifiers = bits;
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0.0, |time| time.as_secs_f64());
            let mut event = serde_json::json!({"type":"keyDown","key":key,"text":text,"modifiers":modifiers,"windowsVirtualKeyCode":virtual_key(&name),"code":code(&name),"timestamp":timestamp});
            sent.store(true, Ordering::Release);
            let release = channel.clone();
            let params = event.to_string();
            event["type"] = serde_json::json!("keyUp");
            if let Some(event) = event.as_object_mut() {
                event.remove("text");
            }
            channel.call(
                "",
                "Input.dispatchKeyEvent",
                &params,
                Box::new(move |pressed| {
                    // Always release the dispatched key, even after Stop or a protocol error; never repeat a press.
                    release.call(
                        "",
                        "Input.dispatchKeyEvent",
                        &event.to_string(),
                        Box::new(move |released| {
                            let _ = reply.send(pressed.and(released).map(|_| ()));
                        }),
                    );
                }),
            );
        }
    }
}
fn virtual_key(key: &str) -> u32 {
    match key {
        "Backspace" => 8,
        "Tab" => 9,
        "Enter" => 13,
        "Shift" => 16,
        "Control" => 17,
        "Alt" => 18,
        "Escape" => 27,
        "Space" => 32,
        "PageUp" => 33,
        "PageDown" => 34,
        "End" => 35,
        "Home" => 36,
        "ArrowLeft" => 37,
        "ArrowUp" => 38,
        "ArrowRight" => 39,
        "ArrowDown" => 40,
        "Insert" => 45,
        "Delete" => 46,
        "Meta" => 91,
        _ => key
            .chars()
            .next()
            .filter(char::is_ascii)
            .map_or(0, |c| u32::from(c.to_ascii_uppercase())),
    }
}

fn code(key: &str) -> String {
    if key.len() == 1 && key.as_bytes()[0].is_ascii_alphabetic() {
        return format!("Key{}", key.to_ascii_uppercase());
    }
    if key.len() == 1 && key.as_bytes()[0].is_ascii_digit() {
        return format!("Digit{key}");
    }
    match key {
        "Control" => "ControlLeft",
        "Shift" => "ShiftLeft",
        "Alt" => "AltLeft",
        "Meta" => "MetaLeft",
        other => other,
    }
    .into()
}
