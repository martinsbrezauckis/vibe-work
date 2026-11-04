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

/// A screenshot-free check of the requested surface using an existing tool.
pub(crate) fn text_check(app_preview: bool) -> &'static str {
    if app_preview {
        "check App preview errors with preview_console"
    } else {
        "check without a screenshot (browser_read text)"
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
    let check = text_check(target == owner);
    if let Some(report) = report {
        if !report.window_visible {
            return Some(window_closed_text(label, target == owner));
        }
        if report.selected_session_id.is_none() {
            return Some(format!(
                "The user is viewing another page, so {label} cannot be captured. Ask the user to open this session, or {check}."
            ));
        }
        if report.selected_session_id.as_deref() != Some(owner) && on_new_chat {
            return Some(format!(
                "No chat is selected, so {label} cannot be captured. Ask the user to open this session, or {check}."
            ));
        }
        if report.selected_session_id.as_deref() != Some(owner) {
            return Some(format!(
                "The user is viewing another session, so {label} cannot be captured. Ask the user to open this session, or {check}."
            ));
        }
        if !report.browser_panel_open {
            return Some(format!(
                "The Browser side panel is closed or shows another panel tab, so {label} cannot be captured. Ask the user to open Browser, or {check}."
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
            "The user has selected {selected} instead of {label}. Ask the user to select {label}, or {check}."
        ));
    }
    (!native_shown).then(|| {
        format!("{label} is not shown in the Browser panel. Ask the user to show it, or {check}.")
    })
}

/// The same reason whether native state or the webview reported the hidden window.
pub(crate) fn window_closed_text(label: &str, app_preview: bool) -> String {
    #[cfg(target_os = "macos")]
    let text = vw_vocabulary::texts::CAPTURE_WINDOW_CLOSED_MAC;
    #[cfg(not(target_os = "macos"))]
    let text = vw_vocabulary::texts::CAPTURE_WINDOW_CLOSED;
    text.replace("{label}", label).replace(
        "check without a screenshot (browser_read text)",
        text_check(app_preview),
    )
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
        let place = if cfg!(target_os = "macos") {
            "menu bar"
        } else {
            "tray"
        };
        assert!(
            screenshot_reason(Some(&r), "s", "s", "App preview", true, false, false)
                .unwrap()
                .contains(&format!("window is closed to the {place} or minimized"))
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

#[cfg(test)]
mod fallback_tests {
    #[test]
    fn hidden_app_preview_names_its_own_text_tool() {
        let report = super::VisibilityReport {
            window_visible: false,
            selected_session_id: Some("s".into()),
            browser_panel_open: true,
            selected_page_key: Some("s".into()),
        };
        let reason =
            super::screenshot_reason(Some(&report), "s", "s", "App preview", false, true, false)
                .unwrap();
        assert!(
            reason.contains("preview_console") && !reason.contains("browser_read"),
            "{reason}"
        );
        let web =
            super::screenshot_reason(Some(&report), "s", "web", "web tab", false, true, false)
                .unwrap();
        assert!(web.contains("browser_read"), "{web}");
    }
}
