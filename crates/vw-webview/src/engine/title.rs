//! Reads the title and URL from one document in the preview webview.
//!
//! Depends on: Tauri's native script callback and preview hooks. Called by page-load and title-change callbacks.

use std::sync::Arc;

use tauri::{Webview, Wry};
use url::Url;
use vw_preview::{PageEvent, PreviewHooks};

const DOCUMENT: &str =
    "[String(location.href), String(document.title), document.readyState === 'complete']";

fn pair(json: &str) -> Option<(Url, String, bool)> {
    let (url, text, complete): (String, String, bool) = serde_json::from_str(json).ok()?;
    Some((Url::parse(&url).ok()?, text, complete))
}

fn completed(json: &str, expected: &Url) -> Option<(Url, String)> {
    let (url, text, ready) = pair(json)?;
    (ready && &url == expected).then_some((url, text))
}

/// A delayed title callback reads the current document, not the callback's stale text.
pub(super) fn report(webview: &Webview<Wry>, hooks: &Arc<dyn PreviewHooks>, label: &str) {
    let hooks = hooks.clone();
    let label = label.to_owned();
    let _ = webview.eval_with_callback(DOCUMENT, move |json| {
        if let Some((url, text, _)) = pair(&json) {
            let _ = hooks.page_event(&label, PageEvent::TitleAt { url, text });
        }
    });
}

/// Finish only when the loaded document itself reports the expected URL and complete readiness.
pub(super) fn finish(
    webview: &Webview<Wry>,
    hooks: &Arc<dyn PreviewHooks>,
    label: &str,
    expected: Url,
) {
    let hooks = hooks.clone();
    let label = label.to_owned();
    let unload = webview.clone();
    let _ = webview.eval_with_callback(DOCUMENT, move |json| {
        let Some((url, text)) = completed(&json, &expected) else {
            return;
        };
        let _ = hooks.page_event(
            &label,
            PageEvent::TitleAt {
                url: url.clone(),
                text,
            },
        );
        if !hooks.page_event(&label, PageEvent::Finished(url))
            && let Ok(blank) = Url::parse("about:blank")
        {
            let _ = unload.navigate(blank);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::completed;
    use url::Url;

    #[test]
    fn only_complete_matching_document_finishes() {
        let expected = Url::parse("http://127.0.0.1:30101/new").unwrap();
        assert!(completed("[\"http://127.0.0.1:30101/old\",\"old\",true]", &expected).is_none());
        assert!(completed("[\"http://127.0.0.1:30101/new\",\"new\",false]", &expected).is_none());
        assert!(completed("[\"http://127.0.0.1:30101/new\",\"new\",true]", &expected).is_some());
    }
}
