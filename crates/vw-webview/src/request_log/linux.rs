//! `WebKitGTK` resource events record requests, final response metadata and real transport errors.
//! Installed before the first navigation; callbacks retain only the bounded per-webview log.
use super::{Shared, lock};
use serde_json::{Value, json};
use webkit2gtk::{URIRequestExt, URIResponseExt, WebResourceExt, WebViewExt};
fn response(resource: &webkit2gtk::WebResource) -> Value {
    let Some(response) = resource.response() else {
        return json!({});
    };
    let mut headers = serde_json::Map::new();
    let mut truncated = false;
    if let Some(native) = response.http_headers() {
        native.foreach(|name, value| {
            if headers.len() < 32 {
                headers.insert(
                    name.chars().take(128).collect(),
                    json!(value.chars().take(512).collect::<String>()),
                );
                truncated |= value.len() > 512;
            } else {
                truncated = true;
            }
        });
    }
    json!({"status":response.status_code(),"finalUrl":response.uri().map(|s|s.to_string()),"responseHeaders":headers,"responseHeadersTruncated":truncated})
}
pub(super) fn install(view: &webkit2gtk::WebView, records: Shared) {
    let reset = records.clone();
    // The address of the page being left, read when a main-frame load starts (before the view's address moves).
    let shown = std::rc::Rc::new(std::cell::RefCell::new(None::<String>));
    let (seen, restore) = (shown.clone(), shown);
    view.connect_load_changed(move |view, event| {
        if event == webkit2gtk::LoadEvent::Started {
            let mut log = lock(&reset);
            log.reset();
            log.sources().load_started(seen.borrow_mut().take());
        } else if event == webkit2gtk::LoadEvent::Committed {
            let address = view.uri().map(|uri| uri.to_string());
            lock(&reset).sources().committed(address.as_deref());
            *restore.borrow_mut() = address;
        }
    });
    view.connect_resource_load_started(move |view,resource,request|{
        let started_at_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .and_then(|elapsed| u64::try_from(elapsed.as_millis()).ok());
        if view.main_resource().as_ref()==Some(resource){lock(&records).sources().main_requested();}
        let mut headers=serde_json::Map::new();
        let mut truncated=false;
        if let Some(native)=request.http_headers(){native.foreach(|name,value|{
            if headers.len()<32 {headers.insert(name.chars().take(128).collect(),json!(value.chars().take(512).collect::<String>()));truncated|=value.len()>512;} else {truncated=true;}
        });}
        let id=lock(&records).start(json!({"url":request.uri().map(|s|s.to_string()),"method":request.http_method().map(|s|s.to_string()),"requestHeaders":headers,"requestHeadersTruncated":truncated,"finished":false,"startedAtMs":started_at_ms}));
        // Each HTML response counts once while arriving and settles once (`finished` follows `failed`).
        let settle=std::rc::Rc::new(std::cell::Cell::new(Settle::default()));
        let (log,key,state)=(records.clone(),id.clone(),settle.clone());
        resource.connect_response_notify(move |resource|{
            lock(&log).update(&key,response(resource));
            // HTML counts as holding a closed declarative root until its source is checked.
            let mut now=state.get();
            if html(resource)&&!now.counted{now.counted=true;state.set(now);lock(&log).sources().arriving();}
        });
        let (log,key)=(records.clone(),id.clone());
        let state=settle.clone();
        resource.connect_failed(move |_,error|{
            lock(&log).update(&key,json!({"error":error.to_string().chars().take(500).collect::<String>(),"finished":true}));
            let mut now=state.get();
            now.failed=Some(error.matches(webkit2gtk::NetworkError::Cancelled));
            state.set(now);
        });
        let (log,key)=(records.clone(),id);
        resource.connect_finished(move |resource|{
            let mut fields=response(resource);fields["finished"]=json!(true);lock(&log).update(&key,fields);
            keep_source(resource,&log,settle.get());
        });
    });
}

/// One HTML response's settling: whether its arrival was counted, and how it failed (`Some(true)`: cancelled).
#[derive(Clone, Copy, Default)]
struct Settle {
    counted: bool,
    failed: Option<bool>,
}

/// An HTML response's source (a page, a frame, or HTML a page fetched), checked for closed declarative shadow roots
/// once it has arrived (`finished`, which also follows a failure). A cancelled load (`window.stop()` keeps what was
/// parsed) is checked on what arrived; any other failure, or a source that cannot be read, counts as holding one.
fn keep_source(resource: &webkit2gtk::WebResource, log: &Shared, settle: Settle) {
    if !html(resource) {
        return;
    }
    let settled = move |log: &Shared, verdict: bool| {
        let mut log = lock(log);
        if settle.counted {
            log.sources().arrived(Some(verdict));
        } else {
            log.sources().checked(verdict);
        }
    };
    if settle.failed == Some(false) {
        settled(log, true);
        return;
    }
    let log = log.clone();
    resource.data(None::<&webkit2gtk::gio::Cancellable>, move |data| {
        let verdict = data.map_or(true, |bytes| vw_preview::declarative_closed_source(&bytes));
        settled(&log, verdict);
    });
}

/// An HTML response (by its MIME type, after the engine's sniffing).
fn html(resource: &webkit2gtk::WebResource) -> bool {
    resource
        .response()
        .and_then(|response| response.mime_type())
        .is_some_and(|mime| matches!(mime.as_str(), "text/html" | "application/xhtml+xml"))
}
