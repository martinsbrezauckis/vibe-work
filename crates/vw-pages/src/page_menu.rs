//! The Project page's right click: the page script (`page_menu.js`) keeps the last trusted right click; a native
//! poll takes it as a point and a "text was selected" flag only and hands it to the app, which opens its own menu.
//! The menu's items then read what the right click was on: the section (or the selection) for Point Vibe at this
//! and Comment…, the selected text for Copy. Also the selection-only read of a web page in the Browser tab.
//!
//! Depends on: the viewer's engine (`PagesService::engine`), `pick` (the shared `describe`), tokio. Called by:
//! `service` (starts the poll with the viewer's mount), vw-core-api (`pages` menu requests, the Browser tab's point).
//! Invariants:
//! - The poll answer is untrusted page data: parsed as JSON only, the point bounded, anything else is "no click".
//! - No page text crosses before a person chooses an item; what an item reads is bounded again here.
//! - The poll ends when its mount closes; it never runs for the File tab's viewer.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::config::{PagesError, PickedSection};
use crate::paths::PagePath;
use crate::pick::{DESCRIBE_JS, PickAnswer, Picked, parse};
use crate::picking::section_of;
use crate::service::{PagesService, bounded, mount_id};

/// How often the viewer is asked for a right click.
const POLL: Duration = Duration::from_millis(150);
/// Largest coordinate accepted (CSS pixels of the page's viewport).
const MAX_POINT: f64 = 20_000.0;
/// Longest text Copy takes from the page.
pub const MAX_COPY: usize = 100_000;

/// A person right-clicked the page shown in `session_id`'s Project tab at (`x`, `y`) (CSS pixels of the page's
/// viewport); `selected` when the click was on selected text.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageMenuAt {
    /// The session whose page it was.
    pub session_id: String,
    /// Left of the click.
    pub x: f64,
    /// Top of the click.
    pub y: f64,
    /// The click was on selected text.
    pub selected: bool,
}

/// The page script, installed at document start in the viewer's top document (with the inline controls).
#[must_use]
pub fn page_script() -> &'static str {
    include_str!("page_menu.js")
}

const POLL_SCRIPT: &str = "(typeof window.__vw_page_menu==='object' && window.__vw_page_menu) ? window.__vw_page_menu.take() : ''";

#[derive(Deserialize)]
struct Click {
    x: f64,
    y: f64,
    selected: bool,
}

/// The point and flag of one poll answer (the JSON of the returned string), or `None` for no click.
#[must_use]
pub fn parse_click(raw: &str) -> Option<(f64, f64, bool)> {
    if raw.len() > 1024 {
        return None;
    }
    let inner: String = serde_json::from_str(raw).ok()?;
    let click: Click = serde_json::from_str(&inner).ok()?;
    let ok = |v: f64| v.is_finite() && (-MAX_POINT..=MAX_POINT).contains(&v);
    (ok(click.x) && ok(click.y)).then(|| (click.x.max(0.0), click.y.max(0.0), click.selected))
}

/// The script that describes what the last right click was on: the selection (as the quote) when it was on
/// selected text, else the element under the pointer. The answer has the shape of a click pick.
#[must_use]
pub fn target_script() -> String {
    format!(
        r#"(() => {{
{DESCRIBE_JS}
  const t = (typeof window.__vw_page_menu === "object" && window.__vw_page_menu) ? window.__vw_page_menu.target() : null;
  if (!t || !t.element || !t.element.isConnected) return JSON.stringify({{ state: "cancelled" }});
  if (t.range) {{
    let node = t.range.commonAncestorContainer;
    if (node.nodeType !== 1) node = node.parentElement;
    return JSON.stringify(describe(node || document.body, t.text));
  }}
  return JSON.stringify(describe(t.element, ""));
}})()"#
    )
}

/// The script that answers the text the last right click had selected, line breaks kept, at most [`MAX_COPY`]
/// characters.
#[must_use]
pub fn copy_script() -> String {
    format!(
        "(() => {{ const t = (typeof window.__vw_page_menu === 'object' && window.__vw_page_menu) ? \
         window.__vw_page_menu.target() : null; return t && t.range ? String(t.text).slice(0, {MAX_COPY}) : ''; }})()"
    )
}

/// The script that describes the page's selection now, or answers "cancelled" without one: never an overlay, never
/// a wait for a click (a web page in the Browser tab, after its own menu's Point Vibe at this).
#[must_use]
pub fn selection_script() -> String {
    format!(
        r#"(() => {{
{DESCRIBE_JS}
  const sel = window.getSelection && window.getSelection();
  if (!sel || sel.isCollapsed || !String(sel).trim()) return JSON.stringify({{ state: "cancelled" }});
  let node = sel.getRangeAt(0).commonAncestorContainer;
  if (node.nodeType !== 1) node = node.parentElement;
  return JSON.stringify(describe(node || document.body, String(sel)));
}})()"#
    )
}

/// The text of a [`copy_script`] answer, bounded again.
#[must_use]
pub fn parse_copy(raw: &str) -> String {
    if raw.len() > MAX_COPY * 8 {
        return String::new();
    }
    serde_json::from_str::<String>(raw)
        .unwrap_or_default()
        .chars()
        .take(MAX_COPY)
        .collect()
}

impl PagesService {
    /// Right clicks on the Project page, as they happen (point and flag only).
    #[must_use]
    pub fn page_menus(&self) -> tokio::sync::broadcast::Receiver<PageMenuAt> {
        self.shared.menus.subscribe()
    }

    /// Polls the mounted viewer `label` for a right click until that mount closes.
    pub(crate) fn start_menu_poll(&self, label: String) {
        let service = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(POLL);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tick.tick().await;
                let Some(mount) = service.shared.lock().mount.clone() else {
                    return;
                };
                if mount.label != label {
                    return;
                }
                let answer = bounded(
                    service.config.engine_timeout,
                    "page menu",
                    service
                        .engine
                        .evaluate(&mount_id(&mount), POLL_SCRIPT.to_owned()),
                )
                .await;
                let Some((x, y, selected)) = answer.ok().as_deref().and_then(parse_click) else {
                    continue;
                };
                // The session the viewer shows now (the core's record, never the page's word).
                let shown = service
                    .shared
                    .lock()
                    .mount
                    .as_ref()
                    .map(|m| m.session.clone());
                if let Some(session_id) = shown.filter(|_| mount.label == label) {
                    let _ = service.shared.menus.send(PageMenuAt {
                        session_id,
                        x,
                        y,
                        selected,
                    });
                }
            }
        });
    }

    /// The section (with the selection as its quote, when there was one) the last right click on `session`'s page
    /// was on.
    ///
    /// # Errors
    /// [`PagesError::NothingPicked`] when the page holds no right click, [`PagesError::NoSection`], engine failures.
    pub async fn menu_section(
        &self,
        session: &str,
        project: &std::path::Path,
    ) -> Result<PickedSection, PagesError> {
        let picked = self.menu_target(session).await?;
        self.menu_section_from_pick(session, project, picked).await
    }

    /// The menu's section read for an owner who names a section and optional exact selected text.
    /// No page input event is created; the selection must occur in the named element.
    ///
    /// # Errors
    /// A missing mount, target or section, or an engine/page-read failure.
    pub async fn menu_section_named(
        &self,
        session: &str,
        project: &std::path::Path,
        section: &str,
        quote: Option<&str>,
    ) -> Result<PickedSection, PagesError> {
        let id = self.menu_mount(session)?;
        let script = crate::target_script_with_quote(&named_section(section), quote);
        let answer = bounded(
            self.config.engine_timeout,
            "named page menu target",
            self.engine.evaluate(&id, script),
        )
        .await?;
        let PickAnswer::Picked(picked) = parse(&answer) else {
            return Err(PagesError::NamedTargetMissing);
        };
        self.menu_section_from_pick(session, project, picked).await
    }

    async fn menu_section_from_pick(
        &self,
        session: &str,
        project: &std::path::Path,
        picked: Picked,
    ) -> Result<PickedSection, PagesError> {
        let path = self
            .view(session)
            .and_then(|v| v.path)
            .ok_or_else(|| PagesError::NoPage("(the hub)".to_owned()))?;
        let page = PagePath::parse(&path)?;
        let (service, project_owned, page_owned) =
            (self.clone(), project.to_path_buf(), page.clone());
        let facts = tokio::task::spawn_blocking(move || service.page(&project_owned, &page_owned))
            .await
            .map_err(|e| PagesError::Other(e.to_string()))??;
        section_of(&facts, &page, &picked).ok_or(PagesError::NoSection)
    }

    /// The viewer while it holds `session`'s page, shown or parked (the app parks it while its menu is open).
    fn menu_mount(&self, session: &str) -> Result<vw_preview::MountId, PagesError> {
        match self.shared.lock().mount.as_ref() {
            Some(mount) if mount.session == session => Ok(mount_id(mount)),
            _ => Err(PagesError::NotShown),
        }
    }

    async fn menu_target(&self, session: &str) -> Result<Picked, PagesError> {
        let id = self.menu_mount(session)?;
        let answer = bounded(
            self.config.engine_timeout,
            "page menu target",
            self.engine.evaluate(&id, target_script()),
        )
        .await?;
        match parse(&answer) {
            PickAnswer::Picked(picked) => Ok(picked),
            _ => Err(PagesError::NothingPicked),
        }
    }

    /// The text the last right click on `session`'s page had selected (empty without a selection).
    ///
    /// # Errors
    /// Engine failures; [`PagesError::NotShown`] when the viewer holds another session's page.
    pub async fn menu_copy(&self, session: &str) -> Result<String, PagesError> {
        let id = self.menu_mount(session)?;
        let answer = bounded(
            self.config.engine_timeout,
            "page menu copy",
            self.engine.evaluate(&id, copy_script()),
        )
        .await?;
        Ok(parse_copy(&answer))
    }

    /// The menu's Copy read for a named section, with the same text bound and line breaks.
    ///
    /// # Errors
    /// A missing mount or target, or a failed engine evaluation.
    pub async fn menu_copy_named(
        &self,
        session: &str,
        section: &str,
        quote: Option<&str>,
    ) -> Result<String, PagesError> {
        let id = self.menu_mount(session)?;
        let script = crate::copy_target_script(&named_section(section), quote);
        let answer = bounded(
            self.config.engine_timeout,
            "named page menu copy",
            self.engine.evaluate(&id, script),
        )
        .await?;
        crate::parse_named_copy(&answer).ok_or(PagesError::NamedTargetMissing)
    }
}

fn named_section(section: &str) -> crate::PickAt {
    let name = serde_json::json!(section);
    crate::PickAt::Selector(format!("[id={name}], [data-step={name}]"))
}

#[cfg(test)]
mod tests {
    use super::{MAX_COPY, parse_click, parse_copy};

    fn answer(inner: &str) -> String {
        serde_json::Value::String(inner.to_owned()).to_string()
    }

    #[test]
    fn a_poll_answer_is_a_bounded_point_or_nothing() {
        assert_eq!(
            parse_click(&answer(r#"{"x":40,"y":12.5,"selected":true}"#)),
            Some((40.0, 12.5, true))
        );
        assert_eq!(
            parse_click(&answer(r#"{"x":-3,"y":7,"selected":false}"#)),
            Some((0.0, 7.0, false))
        );
        // No click, a page's other shapes, out-of-range points: no menu.
        assert_eq!(parse_click(&answer("")), None);
        assert_eq!(parse_click("\"\""), None);
        assert_eq!(parse_click(r#"{"x":1,"y":1,"selected":true}"#), None);
        assert_eq!(
            parse_click(&answer(r#"{"x":1e9,"y":1,"selected":true}"#)),
            None
        );
        assert_eq!(
            parse_click(&answer(r#"{"x":"1","y":1,"selected":true}"#)),
            None
        );
        assert_eq!(
            parse_click(&answer(&format!(
                "{{\"x\":1,\"y\":1,\"selected\":true,\"pad\":\"{}\"}}",
                "a".repeat(2000)
            ))),
            None
        );
    }

    #[test]
    fn copy_keeps_line_breaks_and_is_bounded() {
        assert_eq!(parse_copy(&answer("one\ntwo")), "one\ntwo");
        assert_eq!(parse_copy("null"), "");
        let long = answer(&"x".repeat(MAX_COPY + 50));
        assert_eq!(parse_copy(&long).chars().count(), MAX_COPY);
    }
}
