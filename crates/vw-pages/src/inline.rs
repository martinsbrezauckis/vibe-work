//! Trusted viewer controls and the prototype host. Depends on the native engine, answers and page tokenizer.
//! Called by service/server. Sandbox messages are geometry or picks only; answer writes use the existing store.

use std::fmt::Write as _;
use std::sync::atomic::Ordering;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::answers::{AnswerValue, PageAnswers};
use crate::html::{Token, escape, tokens};
use crate::paths::PagePath;
use crate::service::{PagesService, bounded, mount_id};

/// Native document-start bootstrap; never installed in the File tab's script-capable viewer.
pub(crate) fn bootstrap() -> String {
    // The bundled sheet supplies fallback tokens only. Its page-layout rules must not change authored layouts.
    let tokens = crate::template::STYLE
        .lines()
        .filter(|s| s.starts_with(":root") || s.starts_with("@media"))
        .collect::<Vec<_>>()
        .join("\n");
    let css = format!("{tokens}\n{}", include_str!("inline_controls.css"));
    format!(
        "if(window.top===window && window.origin!=='null'){{window.__hbInlineStyle={};}}\n{}\n{}\n{}",
        serde_json::Value::String(css),
        include_str!("anchors.js"),
        include_str!("inline_controls.js"),
        crate::page_menu::page_script()
    )
}

/// The trusted parent retains authored styles for control hooks, but runs no authored script.
pub(crate) fn prototype_host(source: &str, title: &str) -> Vec<u8> {
    let mut styles = String::new();
    for token in tokens(source) {
        match token {
            Token::Style(css) => {
                let _ = write!(styles, "<style>{css}</style>");
            }
            Token::Tag(tag) if tag.name == "link" && tag.attr("rel") == Some("stylesheet") => {
                if let Some(href) = tag.attr("href") {
                    let _ = write!(
                        styles,
                        "<link rel=\"stylesheet\" href=\"{}\">",
                        escape(href)
                    );
                }
            }
            _ => {}
        }
    }
    format!("<!doctype html><html><head><meta charset=\"utf-8\"><title>{}</title>{styles}\
        <style>html,body{{margin:0!important;padding:0!important;width:100%;height:100%;overflow:hidden!important}}\
        iframe[data-hb-prototype]{{position:fixed;inset:0;width:100%;height:100%;border:0;display:block}}</style>\
        </head><body><iframe data-hb-prototype sandbox=\"allow-scripts\" title=\"{}\" src=\"?hb-body\"></iframe></body></html>", escape(title), escape(title)).into_bytes()
}

/// Prelude of the sandboxed frame (layout and existing handbook comments, never answer writes).
pub(crate) fn prelude() -> String {
    format!(
        "<script>{}\n{}</script>",
        include_str!("anchors.js"),
        include_str!("inline_page.js")
    )
}

/// A prototype frame may load only its owning page. Changing documents must never inherit its answer channel.
pub(crate) fn host_policy(origin: &str, path: &str) -> String {
    let mut encoded = String::new();
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || b"/%-_.~".contains(&byte) {
            encoded.push(char::from(byte));
        } else {
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    crate::csp::Policy::InlineHost
        .header(origin)
        .replace("frame-src 'self'", &format!("frame-src {origin}{encoded}"))
        .replace("child-src 'self'", &format!("child-src {origin}{encoded}"))
}

#[derive(Clone, Deserialize, Serialize)]
struct Change {
    input: String,
    value: Option<AnswerValue>,
    sequence: u64,
}

#[derive(Deserialize)]
struct Poll {
    document: String,
    path: String,
    ready: bool,
    initialized: bool,
    changes: Vec<Change>,
}

fn parse_poll(raw: &str) -> Option<Poll> {
    if raw.len() > 2 * 1024 * 1024 {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(raw).ok()?;
    let poll: Poll = match value {
        serde_json::Value::String(text) => serde_json::from_str(&text).ok()?,
        other => serde_json::from_value(other).ok()?,
    };
    (poll.changes.len() <= 500).then_some(poll)
}

impl PagesService {
    /// Drains committed gestures before a native navigation destroys their document or its debounce timer.
    pub(crate) async fn flush_inline(
        &self,
        mount: &crate::viewer::Mount,
    ) -> Result<(), crate::config::PagesError> {
        if self.config.pdf {
            return Ok(());
        }
        let Some(path) = &mount.path else {
            return Ok(());
        };
        let result = bounded(
            self.config.engine_timeout,
            "flush inline answers",
            self.engine.evaluate(
                &mount_id(mount),
                "typeof window.__hbInline==='function'?window.__hbInline('flush'):null".into(),
            ),
        )
        .await;
        if let Some(poll) = result.ok().and_then(|raw| parse_poll(&raw))
            && PagePath::parse(&poll.path).ok().as_ref() == Some(path)
            && !poll.changes.is_empty()
        {
            if let Some(page) = self
                .apply_inline(&mount.project, path, poll.changes.clone())
                .await
                .map_err(crate::config::PagesError::Other)?
            {
                let data = serde_json::json!({ "document": poll.document, "page": page, "ack": poll.changes });
                let _ = bounded(self.config.engine_timeout, "ack inline answers", self.engine.evaluate(&mount_id(mount),
                    format!("typeof window.__hbInline==='function'?window.__hbInline('sync',{data}):null"))).await;
            }
            self.shared.changed(Some(&mount.session));
        }
        Ok(())
    }
    async fn apply_inline(
        &self,
        project: &std::path::Path,
        path: &PagePath,
        changes: Vec<Change>,
    ) -> Result<Option<PageAnswers>, String> {
        let (owned, project, path) = (self.clone(), project.to_path_buf(), path.clone());
        tokio::task::spawn_blocking(move || {
            let mut last = None;
            for change in changes {
                last = Some(
                    match change.value {
                        Some(value) => owned.set_answer(&project, &path, &change.input, &value),
                        None => owned.clear_answer(&project, &path, &change.input),
                    }
                    .map_err(|e| e.to_string())?,
                );
            }
            Ok(last)
        })
        .await
        .map_err(|e| e.to_string())?
    }

    fn inline_target(
        &self,
        label: &str,
    ) -> Option<(crate::viewer::Mount, std::path::PathBuf, PagePath, u64)> {
        let state = self.shared.lock();
        let mount = state.mount.as_ref()?.clone();
        if mount.label != label {
            return None;
        }
        {
            let viewer = state.viewers.get(&mount.session);
            viewer
                // Native hiding may deliver blur after the pre-hide drain. Finish those already trusted
                // gestures while parked; unchanged snapshots perform no per-input work.
                .filter(|v| !v.loading)
                .and_then(|v| {
                    v.path.clone().map(|p| {
                        (
                            mount.clone(),
                            v.project.clone(),
                            p,
                            *self.shared.loads.borrow(),
                        )
                    })
                })
        }
    }

    async fn inline_answers(
        &self,
        project: &std::path::Path,
        path: &PagePath,
    ) -> Option<PageAnswers> {
        let (owned, project, path) = (self.clone(), project.to_path_buf(), path.clone());
        tokio::task::spawn_blocking(move || owned.answers_of(&project, &path))
            .await
            .ok()?
            .ok()
    }

    /// One small native drain for the mounted viewer; ends when that mount closes. File previews never use it.
    pub(crate) fn start_inline(&self, label: String) {
        let service = self.clone();
        tokio::spawn(async move {
            let mut cache: Option<((String, String, u64), PageAnswers, u64)> = None;
            let mut tick = tokio::time::interval(Duration::from_millis(75));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tick.tick().await;
                if service
                    .shared
                    .lock()
                    .mount
                    .as_ref()
                    .is_none_or(|m| m.label != label)
                {
                    return;
                }
                service.refresh_document(&label).await;
                let target = service.inline_target(&label);
                let Some((mount, project, path, load)) = target else {
                    continue;
                };
                let key = (mount.session.clone(), path.to_string(), load);
                let revision = service.answer_revision.load(Ordering::Acquire);
                let changed = cache
                    .as_ref()
                    .is_none_or(|(k, _, r)| *k != key || *r != revision);
                if changed {
                    let Some(page) = service.inline_answers(&project, &path).await else {
                        continue;
                    };
                    cache = Some((key.clone(), page, revision));
                }
                let Some((_, page, _)) = cache.as_ref() else {
                    continue;
                };
                if page.inputs.is_empty() && !changed {
                    continue;
                }
                let id = mount_id(&mount);
                let result = bounded(
                    service.config.engine_timeout,
                    "inline answers",
                    service.engine.evaluate(
                        &id,
                        "typeof window.__hbInline==='function'?window.__hbInline('poll'):null"
                            .into(),
                    ),
                )
                .await;
                let Some(poll) = result.ok().and_then(|raw| parse_poll(&raw)) else {
                    continue;
                };
                if !poll.ready || PagePath::parse(&poll.path).ok().as_ref() != Some(&path) {
                    continue;
                }
                if page.inputs.is_empty() && !poll.initialized {
                    continue;
                }
                // Navigation/session changes invalidate a drain before any writes. The path always comes from
                // the core's mounted viewer, never from a message sent by the prototype.
                if service.shared.lock().mount.as_ref().is_none_or(|m| {
                    m.label != label || m.session != mount.session || m.path.as_ref() != Some(&path)
                }) {
                    continue;
                }
                if !changed && poll.initialized && poll.changes.is_empty() {
                    continue;
                }
                let mut error = None;
                if !poll.changes.is_empty() {
                    match service
                        .apply_inline(&project, &path, poll.changes.clone())
                        .await
                    {
                        Ok(Some(page)) => cache = Some((key, page, revision)),
                        Ok(None) => {}
                        Err(e) => {
                            error = Some(e);
                            cache = None;
                        }
                    }
                    service.shared.changed(Some(&mount.session));
                }
                let page = if let Some((_, page, _)) = &cache {
                    page.clone()
                } else {
                    let (owned, project, path) = (service.clone(), project.clone(), path.clone());
                    let Ok(Ok(page)) =
                        tokio::task::spawn_blocking(move || owned.answers_of(&project, &path))
                            .await
                    else {
                        continue;
                    };
                    page
                };
                let data = serde_json::json!({ "document": poll.document, "page": page, "ack": poll.changes, "error": error });
                let _ = bounded(service.config.engine_timeout, "sync inline answers", service.engine.evaluate(&id,
                    format!("typeof window.__hbInline==='function'?window.__hbInline('sync',{data}):null"))).await;
            }
        });
    }
}
