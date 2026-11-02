//! Linux frame transport and embedded extension installation. Uses `WebKit` `UserMessage`, never page IPC.
//! Called by the engine builder and frame evaluator; each process uses a fresh private extension directory.

use glib::variant::ToVariant;
use javascriptcore::ValueExt as _;
use std::{io::Write, path::PathBuf, sync::OnceLock, time::Duration};
use tauri::{Webview, Wry};
use vw_preview::{EngineError, FrameAnswer};
use webkit2gtk::{UserMessageExt as _, WebViewExt as _};

pub(crate) fn extension_directory() -> Result<PathBuf, EngineError> {
    static DIRECTORY: OnceLock<Result<PathBuf, String>> = OnceLock::new();
    DIRECTORY
        .get_or_init(|| {
            let path = std::env::temp_dir().join(format!(
                "vw-frames-{}-{}",
                std::process::id(),
                vw_preview::accessor_name()
            ));
            vw_platform::ensure_private_dir(&path).map_err(|e| e.to_string())?;
            let mut file = vw_platform::create_private_new(&path.join("libagent_frames.so"))
                .map_err(|e| e.to_string())?;
            file.write_all(include_bytes!(concat!(
                env!("OUT_DIR"),
                "/libagent_frames.so"
            )))
            .map_err(|e| e.to_string())?;
            Ok(path)
        })
        .clone()
        .map_err(EngineError::Platform)
}

pub(crate) async fn evaluate(
    webview: &Webview<Wry>,
    frame: Option<String>,
    script: String,
) -> Result<Vec<FrameAnswer>, EngineError> {
    crate::platform::with_native(
        webview,
        "evaluating browser frames",
        Duration::from_secs(5),
        move |native, reply| {
            evaluate_native(
                &native.inner(),
                frame,
                script,
                Box::new(move |result| {
                    let _ = reply.send(result);
                }),
            );
        },
    )
    .await
}

/// Evaluates like [`evaluate`], but as part of the person's gesture: the UI process runs the request in the main
/// frame's private world (`webkit_web_view_evaluate_javascript` counts that as a user gesture), and the extension's
/// function there evaluates the frames within it. A window the act's own click opens therefore opens as it would
/// for a person's click; the engine's setting that lets any page open windows stays off.
pub(crate) async fn evaluate_as_input(
    webview: &Webview<Wry>,
    frame: Option<String>,
    script: String,
) -> Result<Vec<FrameAnswer>, EngineError> {
    crate::platform::with_native(
        webview,
        "evaluating browser frames",
        Duration::from_secs(5),
        move |native, reply| {
            let call = format!(
                "{AGENT_INPUT}({}, {})",
                serde_json::Value::String(frame.unwrap_or_default()),
                serde_json::Value::String(script)
            );
            native.inner().evaluate_javascript(
                &call,
                Some(AGENT_WORLD),
                None,
                None::<&webkit2gtk::gio::Cancellable>,
                move |answer| {
                    let _ = reply.send(
                        answer
                            .map_err(|_| {
                                "frame document changed, detached, or could not be evaluated"
                                    .to_owned()
                            })
                            .and_then(|value| input_answers(&value.to_str())),
                    );
                },
            );
        },
    )
    .await
}

/// The private world the frame extension registers (`frame_extension.c`) and the function it defines there.
const AGENT_WORLD: &str = "vibe-work-agent";
const AGENT_INPUT: &str = "__vwAgentInput";

fn input_answers(text: &str) -> Result<Vec<FrameAnswer>, String> {
    let changed = || "frame document changed, detached, or could not be evaluated".to_owned();
    // `{"error": ...}` from the extension, or a page without the extension's function (not loaded yet).
    let serde_json::Value::Array(items) = serde_json::from_str(text).map_err(|_| changed())? else {
        return Err(changed());
    };
    items
        .iter()
        .map(|item| {
            Some(FrameAnswer {
                id: item["id"].as_str()?.to_owned(),
                generation: Some(item["generation"].as_u64()?),
                main: item["main"].as_bool()?,
                url: item["url"].as_str()?.to_owned(),
                origin: None,
                value: item["value"].as_str()?.to_owned(),
            })
        })
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| "invalid frame answer".to_owned())
}

pub(crate) fn evaluate_native(
    view: &webkit2gtk::WebView,
    frame: Option<String>,
    script: String,
    done: Box<dyn FnOnce(Result<Vec<FrameAnswer>, String>)>,
) {
    let params = (frame.unwrap_or_default(), script).to_variant();
    view.send_message_to_page(
        &webkit2gtk::UserMessage::new("agent-evaluate", Some(&params)),
        None::<&webkit2gtk::gio::Cancellable>,
        move |answer| {
            let result = answer.map_err(|e| e.to_string()).and_then(|answer| {
                let params = answer
                    .parameters()
                    .ok_or("frame extension gave no answer")?;
                if answer.name().as_deref() != Some("agent-answers") {
                    return Err(
                        "frame document changed, detached, or could not be evaluated".into(),
                    );
                }
                let values = params
                    .get::<Vec<(String, u64, bool, String, String)>>()
                    .ok_or("invalid frame answer")?;
                Ok(values
                    .into_iter()
                    .map(|(id, generation, main, url, value)| FrameAnswer {
                        id,
                        generation: Some(generation),
                        main,
                        url,
                        origin: None,
                        value,
                    })
                    .collect())
            });
            done(result);
        },
    );
}
