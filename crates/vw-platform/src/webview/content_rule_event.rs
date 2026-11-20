//! Actions a webview's content rules performed, delivered to the watcher of that webview.
//!
//! macOS reports them through `WebKit`'s PRIVATE navigation-delegate method
//! `_webView:contentRuleListWithIdentifier:performedAction:forURL:` (`WKNavigationDelegatePrivate.h`, present since
//! macOS 10.15). `WebKit` calls it for loads its content rules handled in the page's resource loader (subresources,
//! child-frame documents, redirects, pings, beacons and CSP reports); a blocked `WebSocket` connection is not reported.
//! `WebKit` decides whether to call the method when the navigation delegate is set (`respondsToSelector:`), so the
//! method must belong to the delegate's class before `setNavigationDelegate:`. Any macOS update may remove it; nothing
//! is reported then and the rules still hold.
//!
//! Depends on: nothing native here; the macOS delegate (`dialogs::macos`) parses the native action. Called by:
//! vw-webview's request log (registers a sink per webview label).
//! Invariants: the registry lives on the native UI thread (the thread that installs delegates and receives their
//! calls); a label's sink is removed when its webview closes ([`forget`]).

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

/// One content-rule action on one load.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentRuleAction {
    /// Identifier of the rule list that acted (Vibe Work's network-policy list uses the webview label).
    pub list: String,
    /// The load's URL.
    pub url: String,
    /// The rule list stopped the load before anything was sent.
    pub blocked_load: bool,
}

/// Receives a webview's content-rule actions on the native UI thread; must return quickly.
pub type ContentRuleSink = Arc<dyn Fn(ContentRuleAction)>;

thread_local! {
    static SINKS: RefCell<HashMap<String, ContentRuleSink>> = RefCell::new(HashMap::new());
}

/// Sends the content-rule actions of webview `label` to `sink` (replaces an earlier sink of that label).
pub fn watch(label: &str, sink: ContentRuleSink) {
    SINKS.with(|sinks| sinks.borrow_mut().insert(label.to_owned(), sink));
}

/// Stops delivering actions of the closed webview `label`.
pub fn forget(label: &str) {
    SINKS.with(|sinks| sinks.borrow_mut().remove(label));
}

/// Hands `action` to the sink of webview `label`; nothing happens for a webview without one.
pub fn deliver(label: &str, action: ContentRuleAction) {
    let sink = SINKS.with(|sinks| sinks.borrow().get(label).cloned());
    if let Some(sink) = sink {
        sink(action);
    }
}

/// Reads `WebKit`'s `_WKContentRuleListAction` and URL into a [`ContentRuleAction`]; `None` when the action object
/// lacks the `blockedLoad` getter (an unknown interface is never reported as a block).
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
pub(crate) fn from_native(
    identifier: Option<&objc2_foundation::NSString>,
    action: Option<&objc2::runtime::AnyObject>,
    url: Option<&objc2_foundation::NSURL>,
) -> Option<ContentRuleAction> {
    use objc2::{msg_send, sel};
    let action = action?;
    // SAFETY: `respondsToSelector:` on the live action object WebKit passed for this call.
    let readable: bool = unsafe { msg_send![action, respondsToSelector: sel!(blockedLoad)] };
    if !readable {
        return None;
    }
    // SAFETY: the getter exists (checked); `_WKContentRuleListAction.h` declares it `BOOL blockedLoad`.
    let blocked_load: bool = unsafe { msg_send![action, blockedLoad] };
    Some(ContentRuleAction {
        list: identifier.map(ToString::to_string).unwrap_or_default(),
        url: url
            .and_then(objc2_foundation::NSURL::absoluteString)
            .map(|url| url.to_string())
            .unwrap_or_default(),
        blocked_load,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn action(url: &str) -> ContentRuleAction {
        ContentRuleAction {
            list: "preview-1".into(),
            url: url.into(),
            blocked_load: true,
        }
    }

    #[test]
    fn actions_reach_only_their_own_webview_until_it_closes() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let (a, b) = (seen.clone(), seen.clone());
        watch(
            "preview-1",
            Arc::new(move |act| a.lock().unwrap().push(("1", act))),
        );
        watch(
            "preview-2",
            Arc::new(move |act| b.lock().unwrap().push(("2", act))),
        );
        deliver("preview-1", action("http://10.0.0.5:81/"));
        deliver("preview-3", action("http://10.0.0.5:82/"));
        assert_eq!(
            *seen.lock().unwrap(),
            [("1", action("http://10.0.0.5:81/"))]
        );
        forget("preview-1");
        deliver("preview-1", action("http://10.0.0.5:83/"));
        deliver("preview-2", action("http://10.0.0.5:84/"));
        assert_eq!(
            *seen.lock().unwrap(),
            [
                ("1", action("http://10.0.0.5:81/")),
                ("2", action("http://10.0.0.5:84/"))
            ]
        );
        forget("preview-2");
    }
}
