//! Window-reported Browser visibility and the first reason a screenshot cannot be shown.
//! Depends on: serde. Called by the preview service after layout and window state reports.

use serde::{Deserialize, Serialize};

use crate::service::PreviewService;

/// What the main window currently shows, reported whenever its visibility or selection changes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct VisibilityReport {
    /// False when the OS window is minimized or the document is hidden.
    pub window_visible: bool,
    /// The session displayed in the main window, if any.
    pub selected_session_id: Option<String>,
    /// Whether the side panel is open on Browser.
    pub browser_panel_open: bool,
    /// The selected Browser page: app session id or web page key.
    pub selected_page_key: Option<String>,
}

impl PreviewService {
    /// Replaces the window's current visibility facts after a UI state change.
    pub fn report_visibility(&self, report: VisibilityReport) {
        self.report_view(report, |_| false);
    }

    /// [`Self::report_visibility`], where `new_chat` tells whether an id is the New chat screen's owner: the
    /// window reports that id while no chat is selected, and a refusal then says so.
    pub fn report_view(&self, report: VisibilityReport, new_chat: impl Fn(&str) -> bool) {
        let on_new_chat = report.selected_session_id.as_deref().is_some_and(new_chat);
        *self
            .visibility
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some((report, on_new_chat));
        self.visibility_changes
            .send_modify(|revision| *revision = revision.wrapping_add(1));
    }

    /// Native Tauri window state, reported on window events independently of document visibility.
    pub fn report_native_window_visibility(&self, visible: bool) {
        *self
            .native_window_visible
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(visible);
        self.visibility_changes
            .send_modify(|revision| *revision = revision.wrapping_add(1));
    }
}

/// First reason a screenshot of `target` is unavailable. The caller supplies its page label; `on_new_chat`: the
/// reported selection is the New chat screen's owner (no chat is selected).
pub(crate) fn screenshot_reason(
    report: Option<&VisibilityReport>,
    owner: &str,
    target: &str,
    label: &str,
    paused: bool,
    native_shown: bool,
    on_new_chat: bool,
) -> Option<String> {
    let next = "Ask the user to show it, or check without a screenshot (browser_read text).";
    if let Some(report) = report {
        if !report.window_visible {
            return Some(format!(
                "The Vibe Work window is minimized or hidden, so {label} cannot be captured. {next}"
            ));
        }
        if report.selected_session_id.is_none() {
            return Some(format!(
                "The user is viewing another page, so {label} cannot be captured. Ask the user to open this session, or check without a screenshot (browser_read text)."
            ));
        }
        if report.selected_session_id.as_deref() != Some(owner) && on_new_chat {
            return Some(format!(
                "No chat is selected, so {label} cannot be captured. Ask the user to open this session, or check without a screenshot (browser_read text)."
            ));
        }
        if report.selected_session_id.as_deref() != Some(owner) {
            return Some(format!(
                "The user is viewing another session, so {label} cannot be captured. Ask the user to open this session, or check without a screenshot (browser_read text)."
            ));
        }
        if !report.browser_panel_open {
            return Some(format!(
                "The Browser side panel is closed or shows another panel tab, so {label} cannot be captured. Ask the user to open Browser, or check without a screenshot (browser_read text)."
            ));
        }
    }
    if paused {
        return Some(format!(
            "The browser engine for {label} is paused to save memory. Ask the user to show {label} in Browser, then retry the screenshot."
        ));
    }
    if let Some(report) = report
        && report.selected_page_key.as_deref() != Some(target)
    {
        let selected = if target == owner {
            "a web tab"
        } else {
            "another tab"
        };
        return Some(format!(
            "The user has selected {selected} instead of {label}. Ask the user to select {label}, or check without a screenshot (browser_read text)."
        ));
    }
    (!native_shown).then(|| {
        format!(
            "{label} is not shown in the Browser panel. Ask the user to show it, or check without a screenshot (browser_read text)."
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn visible() -> VisibilityReport {
        VisibilityReport {
            window_visible: true,
            selected_session_id: Some("s".into()),
            browser_panel_open: true,
            selected_page_key: Some("s".into()),
        }
    }

    #[test]
    fn first_true_reason_for_app_and_web() {
        let mut r = visible();
        r.window_visible = false;
        assert!(
            screenshot_reason(Some(&r), "s", "s", "App preview", true, false, false)
                .unwrap()
                .contains("window is minimized or hidden")
        );
        r.window_visible = true;
        r.selected_session_id = Some("other".into());
        assert!(
            screenshot_reason(Some(&r), "s", "s", "App preview", true, false, false)
                .unwrap()
                .contains("viewing another session")
        );
        r.selected_session_id = Some("s".into());
        r.browser_panel_open = false;
        assert!(
            screenshot_reason(Some(&r), "s", "s", "App preview", true, false, false)
                .unwrap()
                .contains("Browser side panel is closed")
        );
        r.browser_panel_open = true;
        assert!(
            screenshot_reason(Some(&r), "s", "s", "App preview", true, false, false)
                .unwrap()
                .contains("paused to save memory")
        );
        r.selected_page_key = Some("web".into());
        assert!(
            screenshot_reason(Some(&r), "s", "s", "App preview", false, false, false)
                .unwrap()
                .contains("selected a web tab instead of App preview")
        );
        r.selected_page_key = Some("s".into());
        assert!(
            screenshot_reason(Some(&r), "s", "web", "web tab 'Docs'", false, false, false)
                .unwrap()
                .contains("another tab instead of web tab 'Docs'")
        );
    }

    #[test]
    fn away_reasons_never_suggest_width() {
        for r in [
            VisibilityReport {
                window_visible: false,
                ..visible()
            },
            VisibilityReport {
                selected_session_id: Some("other".into()),
                ..visible()
            },
        ] {
            assert!(
                !screenshot_reason(Some(&r), "s", "s", "App preview", false, false, false)
                    .unwrap()
                    .contains("preview_open with a width")
            );
        }
    }
}
