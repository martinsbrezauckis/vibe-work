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
    view.connect_load_changed(move |_, event| {
        if event == webkit2gtk::LoadEvent::Started {
            lock(&reset).reset();
        }
    });
    view.connect_resource_load_started(move |_,resource,request|{
        let mut headers=serde_json::Map::new();
        let mut truncated=false;
        if let Some(native)=request.http_headers(){native.foreach(|name,value|{
            if headers.len()<32 {headers.insert(name.chars().take(128).collect(),json!(value.chars().take(512).collect::<String>()));truncated|=value.len()>512;} else {truncated=true;}
        });}
        let id=lock(&records).start(json!({"url":request.uri().map(|s|s.to_string()),"method":request.http_method().map(|s|s.to_string()),"requestHeaders":headers,"requestHeadersTruncated":truncated,"finished":false}));
        let (log,key)=(records.clone(),id.clone());
        resource.connect_response_notify(move |resource|lock(&log).update(&key,response(resource)));
        let (log,key)=(records.clone(),id.clone());
        resource.connect_failed(move |_,error|lock(&log).update(&key,json!({"error":error.to_string().chars().take(500).collect::<String>(),"finished":true})));
        let (log,key)=(records.clone(),id);
        resource.connect_finished(move |resource|{
            let mut fields=response(resource);fields["finished"]=json!(true);lock(&log).update(&key,fields);
        });
    });
}
