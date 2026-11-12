//! `WebView2` Network events provide request metadata, response status and native failure text.
//! Observation is enabled and acknowledged before navigation; page execution and mutation are untouched.
use super::{Shared, lock};
use serde_json::{Value, json};
use std::{cell::RefCell, collections::HashMap, rc::Rc};
use vw_platform::webview::devtools_win::{Controller, DevTools};
fn headers(value: &Value) -> Value {
    value.as_object().map_or_else(
        || json!({}),
        |map| {
            json!(
                map.iter()
                    .take(32)
                    .map(|(name, value)| (
                        name.chars().take(128).collect::<String>(),
                        value
                            .as_str()
                            .unwrap_or_default()
                            .chars()
                            .take(512)
                            .collect::<String>()
                    ))
                    .collect::<std::collections::BTreeMap<_, _>>()
            )
        },
    )
}
fn response(row: &Value) -> Value {
    json!({"status":row["status"],"finalUrl":row["url"],"responseHeaders":headers(&row["headers"]),"responseHeadersTruncated":row["headers"].as_object().is_some_and(|m|m.len()>32 || m.values().any(|v|v.as_str().is_some_and(|s|s.len()>512)))})
}
fn record(
    log: &Shared,
    ids: &mut HashMap<String, String>,
    frame: &str,
    method: &str,
    event: &Value,
) {
    let Some(request) = event["requestId"].as_str() else {
        return;
    };
    if method == "Network.requestWillBeSent" {
        if event["type"] == "Document"
            && event["frameId"].as_str() == Some(frame)
            && event.get("redirectResponse").is_none()
        {
            lock(log).reset();
            ids.clear();
        }
        if let Some(redirect) = event.get("redirectResponse")
            && let Some(id) = ids.get(request)
        {
            let mut fields = response(redirect);
            fields["finished"] = json!(true);
            lock(log).update(id, fields);
        }
        if ids.len() >= 512 {
            ids.clear();
        }
        let row = &event["request"];
        let id=lock(log).start(json!({"url":row["url"],"method":row["method"],"type":event["type"],"requestHeaders":headers(&row["headers"]),"requestHeadersTruncated":row["headers"].as_object().is_some_and(|m|m.len()>32 || m.values().any(|v|v.as_str().is_some_and(|s|s.len()>512))),"finished":false}));
        ids.insert(request.into(), id);
    } else if let Some(id) = ids.get(request) {
        let fields = match method {
            "Network.responseReceived" => response(&event["response"]),
            "Network.loadingFailed" => {
                json!({"error":event["errorText"].as_str().unwrap_or("Native request failed").chars().take(500).collect::<String>(),"finished":true})
            }
            "Network.loadingFinished" => json!({"finished":true}),
            _ => return,
        };
        lock(log).update(id, fields);
        if matches!(method, "Network.loadingFailed" | "Network.loadingFinished") {
            ids.remove(request);
        }
    }
}
pub(super) fn install(
    controller: &Controller,
    records: Shared,
    done: Box<dyn FnOnce(Result<(), String>)>,
) {
    let tools = match DevTools::new(controller) {
        Ok(tools) => tools,
        Err(error) => {
            done(Err(error));
            return;
        }
    };
    let ids = Rc::new(RefCell::new(HashMap::<String, String>::new()));
    let frame = Rc::new(RefCell::new(String::new()));
    let main = frame.clone();
    if let Err(error) = tools.listen(
        &[
            "Network.requestWillBeSent",
            "Network.responseReceived",
            "Network.loadingFailed",
            "Network.loadingFinished",
            "Page.frameNavigated",
        ],
        Box::new(move |session, method, text| {
            if !session.is_empty() {
                return;
            }
            let Ok(event) = serde_json::from_str::<Value>(text) else {
                return;
            };
            if method == "Page.frameNavigated" {
                if event["frame"].get("parentId").is_none() {
                    *frame.borrow_mut() = event["frame"]["id"].as_str().unwrap_or_default().into();
                }
            } else {
                record(
                    &records,
                    &mut ids.borrow_mut(),
                    &frame.borrow(),
                    method,
                    &event,
                );
            }
        }),
    ) {
        done(Err(error));
        return;
    }
    let channel = tools.clone();
    tools.call("","Page.getFrameTree","{}",Box::new(move |answer|{
        let frame=answer.and_then(|text|serde_json::from_str::<Value>(&text).map_err(|e|e.to_string()));
        let Ok(frame)=frame else {done(Err("The main frame could not be identified; reload the browser tab before reading requests.".into()));return;};
        *main.borrow_mut()=frame["frameTree"]["frame"]["id"].as_str().unwrap_or_default().into();
        channel.call("","Network.enable",r#"{"maxTotalBufferSize":0,"maxResourceBufferSize":0}"#,Box::new(move |result|done(result.map(|_|()))));
    }));
}
