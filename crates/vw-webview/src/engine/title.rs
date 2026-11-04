//! Reads the title and URL from one document in the preview webview.
//!
//! Depends on: native isolated frame evaluation and preview hooks. Called by page-load and title-change callbacks.

use std::sync::Arc;

use tauri::{Webview, Wry};
use url::Url;
use vw_preview::{PageEvent, PreviewHooks};

const DOCUMENT: &str =
    "[String(location.href), String(document.title), String(document.readyState)]";

/// The document's URL, title and readiness (`loading`, `interactive` or `complete`).
fn pair(json: &str) -> Option<(Url, String, String)> {
    let (url, text, state): (String, String, String) = serde_json::from_str(json).ok()?;
    Some((Url::parse(&url).ok()?, text, state))
}

/// What to do after one read of the document whose load the engine reported finished.
#[derive(Debug, PartialEq, Eq)]
enum Next {
    /// Report the finish with this URL and title.
    Finish(Url, String),
    /// Read again: the expected document is not complete yet, or the read gave nothing usable.
    Again,
    /// Stop without a finish: another document is loaded (a newer navigation reports its own finish), or the
    /// document could not be read at all.
    Stop,
}

/// Pause between reads of a document that is not complete yet.
const AGAIN_AFTER: std::time::Duration = std::time::Duration::from_millis(100);
/// Reads (2 s) before the engine's finished load counts for a document that still says it is loading: a PDF in
/// `WebKitGTK` is drawn in full while its document says `loading` for good. Other pages complete within these reads,
/// so their title is the final one.
const READS: u32 = 20;

fn next(json: &str, expected: &Url, attempt: u32) -> Next {
    match pair(json) {
        Some((url, _, _)) if &url != expected => Next::Stop,
        Some((url, text, state)) if state == "complete" || attempt >= READS => {
            Next::Finish(url, text)
        }
        Some(_) => Next::Again,
        None if attempt < READS => Next::Again,
        None => Next::Stop,
    }
}

/// Reads metadata through the native main-frame world and uses the native address, never a page's URL claim.
async fn document(webview: &Webview<Wry>) -> Option<String> {
    let answers = crate::agent_frames::evaluate(webview, None, vw_preview::main_script(DOCUMENT))
        .await
        .ok()?;
    let answer = vw_preview::main_answer(answers).ok()?;
    let (reported, title, state) = pair(&answer.value)?;
    let native = Url::parse(&answer.url).ok()?;
    if native.origin() != reported.origin() {
        return None;
    }
    serde_json::to_string(&(native.as_str(), title, state)).ok()
}

/// A delayed title callback reads the current native document instead of the callback's stale text.
pub(super) fn report(webview: &Webview<Wry>, hooks: &Arc<dyn PreviewHooks>, label: &str) {
    let (webview, hooks, label) = (webview.clone(), hooks.clone(), label.to_owned());
    tauri::async_runtime::spawn(async move {
        if let Some(json) = document(&webview).await
            && let Some((url, text, _)) = pair(&json)
        {
            let _ = hooks.page_event(&label, PageEvent::TitleAt { url, text });
        }
    });
}

/// Finishes only the matching native document. Missing isolation or a changed page never supplies a title.
pub(super) fn finish(
    webview: &Webview<Wry>,
    hooks: &Arc<dyn PreviewHooks>,
    label: &str,
    expected: Url,
) {
    let (webview, hooks, label) = (webview.clone(), hooks.clone(), label.to_owned());
    tauri::async_runtime::spawn(async move {
        for attempt in 1..=READS {
            let json = document(&webview).await.unwrap_or_default();
            match next(&json, &expected, attempt) {
                Next::Finish(url, text) => {
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
                        let _ = webview.navigate(blank);
                    }
                    return;
                }
                Next::Again => tokio::time::sleep(AGAIN_AFTER).await,
                Next::Stop => return,
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{Next, READS, next};
    use url::Url;

    fn doc(url: &str, state: &str) -> String {
        format!("[\"{url}\",\"title\",\"{state}\"]")
    }

    #[test]
    fn only_complete_matching_document_finishes_at_once() {
        let expected = Url::parse("http://127.0.0.1:30101/new").unwrap();
        assert_eq!(
            next(&doc("http://127.0.0.1:30101/old", "complete"), &expected, 1),
            Next::Stop
        );
        assert!(matches!(
            next(&doc("http://127.0.0.1:30101/new", "complete"), &expected, 1),
            Next::Finish(_, _)
        ));
        assert_eq!(
            next(
                &doc("http://127.0.0.1:30101/new", "interactive"),
                &expected,
                1
            ),
            Next::Again
        );
    }

    #[test]
    fn a_document_that_keeps_saying_loading_finishes_after_the_reads() {
        // A PDF in WebKitGTK: drawn in full while its document says `loading` for good.
        let expected = Url::parse("http://127.0.0.1:30101/report.pdf").unwrap();
        let pdf = doc("http://127.0.0.1:30101/report.pdf", "loading");
        assert_eq!(next(&pdf, &expected, READS - 1), Next::Again);
        assert!(matches!(next(&pdf, &expected, READS), Next::Finish(_, _)));
    }

    #[test]
    fn an_unreadable_document_is_read_again_then_given_up() {
        let expected = Url::parse("http://127.0.0.1:30101/report.pdf").unwrap();
        assert_eq!(next("not json", &expected, 1), Next::Again);
        assert_eq!(next("not json", &expected, READS), Next::Stop);
    }
}
