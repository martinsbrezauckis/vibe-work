//! macOS: the URL content rules of a preview webview. The mount's rule page (`vw_netguard::rules`) is
//! compiled into a `WKContentRuleList` and installed before the first page loads (the mount waits for the compile);
//! a Local mount's list is compiled again when its ports change; the compiled list is removed from the store when
//! the webview closes.
//!
//! Depends on: tauri, vw-netguard (`rules`), vw-platform (`content_rules`), `platform::with_native`. Called by:
//! `engine` (macOS only).
//! Invariants:
//! - The list's identifier is the mount's webview label (unique per mount); one list per webview at a time; only the
//!   newest started compile of a webview installs (`RuleGenerations`).
//! - Every wait for the main thread is bounded ([`RULES_WAIT`]); a failure is reported to the caller, which logs it
//!   and keeps the mount (the navigation policy stays; the layer is then not reported).

use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Webview, Wry};
use vw_netguard::rules::{RuleGenerations, RulePage, content_rules_json};
use vw_preview::EngineError;

use crate::platform;

/// Longest wait for a rule list to compile and apply (`WebKit` compiles a few dozen rules in milliseconds).
pub(crate) const RULES_WAIT: Duration = Duration::from_secs(10);

/// Compiles `page`'s rules for the webview `label` and installs them as its only rule list.
///
/// # Errors
/// The compile error, or a timeout.
pub(crate) async fn apply(
    webview: &Webview<Wry>,
    label: &str,
    page: &RulePage,
    generations: &Arc<RuleGenerations>,
) -> Result<(), EngineError> {
    let (identifier, json) = (label.to_owned(), content_rules_json(page));
    // a compile that finishes after a newer one started for this webview never installs.
    let generation = generations.begin(label);
    let (gens, key) = (generations.clone(), label.to_owned());
    let install_if: Box<dyn Fn() -> bool + Send> =
        Box::new(move || gens.is_latest(&key, generation));
    platform::with_native(
        webview,
        "applying the content rules",
        RULES_WAIT,
        move |native, reply| {
            vw_platform::content_rules::apply_content_rules(
                native.inner(),
                &identifier,
                &json,
                install_if,
                Box::new(move |result| {
                    let _ = reply.send(result);
                }),
            );
        },
    )
    .await
}

/// Removes the compiled list of the closed webview `label` from the store (on the main thread; nothing waits); a
/// compile of it still running never installs.
pub(crate) fn forget(app: &AppHandle<Wry>, label: &str, generations: &RuleGenerations) {
    generations.forget(label);
    let identifier = label.to_owned();
    if let Err(error) = app.run_on_main_thread(move || {
        vw_platform::content_rules::forget_content_rules(&identifier);
    }) {
        tracing::info!(%error, "a preview's compiled content rules were not removed from WebKit's store");
    }
}
