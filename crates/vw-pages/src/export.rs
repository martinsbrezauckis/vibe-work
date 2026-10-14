//! Share a page as one file: a handbook page with its styles and
//! images in one self-contained HTML file for someone who does not use Vibe Work. The file leaves our sandbox and
//! opens in someone else's browser, so it is built as untrusted output:
//!
//! 1. `lol_html` (an HTML5 tokenizer-based rewriter) does our rewrites: stylesheets and `<style>` collected (their
//!    `url()`s and `@import`s inlined), images and media inside the handbook turned into `data:` URLs, links to other
//!    handbook pages kept as text, the user's answers put as text after each input, a prototype's own scripts
//!    collected (never inside `<template>`, `<svg>` or `<noscript>`), every script removed from the body.
//! 2. `ammonia` (an html5ever tree, allowlist) sanitizes that body and serializes it from the parsed tree: no
//!    script, no event handler, no `javascript:` URL, `data:` only for images and media, no `srcdoc`, `<base>`,
//!    `<meta>`, `<iframe>`, `<object>`, `<embed>`, `<form>`, SVG only as drawing elements (no script, no
//!    `foreignObject`), `<template>` and `<noscript>` dropped with their content.
//! 3. The head is written here: the Content-Security-Policy `<meta>` first (no network; script only for a prototype,
//!    and only its own scripts, by a per-file nonce), then charset, viewport, the escaped title and the collected
//!    CSS (`</` escaped).
//!
//! Depends on: `lol_html`, `ammonia`, `html` (escaping, entities), `links` (resolution inside the handbook), `paths`
//! (MIME types), `answers`, base64, getrandom. Called by: vw-core-api (`exportPage`), tests.
//! Invariants: reads only files inside the handbook folder, through a directory capability (`cap-std`: a symlink
//! swapped in during the export cannot lead outside), never writes;
//! at most [`MAX_EMBEDDED`] bytes are inlined, the rest is left out and listed.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use lol_html::html_content::{ContentType, Element};
use lol_html::{RewriteStrSettings, element, rewrite_str, text};
use serde::{Deserialize, Serialize};

use crate::answers::PageAnswers;
use crate::html::{decode_entities, escape};
use crate::links::{Target, resolve};
use crate::paths::PagePath;

/// Most bytes of files read for one export (the page, stylesheets, images, fonts, scripts together).
pub const MAX_EMBEDDED: usize = 20 * 1024 * 1024;
/// Most bytes of one file inlined.
pub const MAX_ASSET: usize = 8 * 1024 * 1024;
/// Most bytes of `data:` URLs written into one export (a repeated asset is read once, but every use is written).
pub const MAX_OUTPUT: usize = 28 * 1024 * 1024;

/// A page exported as one file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Exported {
    /// The file's text.
    pub html: String,
    /// A file name to suggest (`timeline-view.html`).
    pub name: String,
    /// Files inlined (stylesheets, images, fonts, scripts).
    pub inlined: u32,
    /// References left out (missing, outside the handbook, over the size budget), plain words.
    pub left_out: Vec<String>,
}

/// What the rewrite pass collects.
#[derive(Default)]
struct Collected {
    /// The page's CSS in document order, already inlined (stylesheets and `<style>` elements).
    css: String,
    /// The `<style>` element being read (`lol_html` hands text over in chunks).
    style: String,
    title: String,
    /// A prototype's scripts in document order: (text, `type="module"`).
    scripts: Vec<(String, bool)>,
    /// The inline script being read (`lol_html` hands text over in chunks).
    script: String,
    /// Open `<template>`, `<svg>` and `<noscript>` elements: scripts inside them are not the page's scripts.
    inert: u32,
}

/// Resource reading with the size budget (methods in `export_files`).
pub(crate) struct Files<'a> {
    pub(crate) root: &'a Path,
    /// The handbook folder as a directory capability: every read goes through it (see `export_files`).
    pub(crate) dir: cap_std::fs::Dir,
    pub(crate) page: &'a str,
    pub(crate) budget: usize,
    /// `data:` URL bytes still allowed in the output ([`MAX_OUTPUT`]).
    pub(crate) output: usize,
    /// `data:` URLs already made, by handbook path (a repeated asset is read once).
    pub(crate) urls: std::collections::HashMap<String, String>,
    /// Stylesheets on the current `@import` chain (a loop is cut).
    pub(crate) chain: Vec<String>,
    pub(crate) inlined: u32,
    pub(crate) left_out: Vec<String>,
}

/// Exports `page` of the handbook in `root`. `answers` are the page's inputs with the user's answers (frozen as text
/// after each input). A prototype page keeps its own scripts, inlined and allowed by a nonce.
///
/// # Errors
/// A plain-words reason when the page cannot be read or rewritten.
pub fn export(
    root: &Path,
    page: &PagePath,
    answers: Option<&PageAnswers>,
) -> Result<Exported, String> {
    let dir = cap_std::fs::Dir::open_ambient_dir(root, cap_std::ambient_authority())
        .map_err(|e| format!("the handbook folder could not be opened: {e}"))?;
    let bytes = crate::export_files::read_beneath(&dir, page.as_str(), MAX_EMBEDDED)
        .map_err(|e| format!("the page could not be read: {e}"))?;
    let source = String::from_utf8_lossy(&bytes).into_owned();
    let facts = crate::page::parse_html(&source);
    let prototype = facts.kind.as_deref() == Some(crate::handbook::format::PROTOTYPE);
    let files = Rc::new(RefCell::new(Files {
        root,
        dir,
        page: page.as_str(),
        budget: MAX_EMBEDDED,
        output: MAX_OUTPUT,
        urls: std::collections::HashMap::new(),
        chain: Vec::new(),
        inlined: 0,
        left_out: Vec::new(),
    }));
    let collected = Rc::new(RefCell::new(Collected::default()));
    let body = rewrite(&source, prototype, answers, &files, &collected)?;
    let clean = crate::export_clean::sanitizer().clean(&body).to_string();
    let collected = collected.take();
    let title = {
        let raw = decode_entities(collected.title.trim());
        if raw.is_empty() {
            facts.title.unwrap_or_else(|| page.to_string())
        } else {
            raw
        }
    };
    let nonce = crate::export_clean::nonce()?;
    let files = Rc::try_unwrap(files)
        .map_err(|_| "the export is still in use".to_owned())?
        .into_inner();
    // What could not be inlined is said in the file itself (and returned for the user and the agent).
    let body = crate::export_clean::with_note(&clean, &files.left_out);
    let html = crate::export_clean::document(
        &title,
        &collected.css,
        &body,
        prototype.then_some((&nonce, collected.scripts.as_slice())),
    );
    Ok(Exported {
        html,
        name: page
            .as_str()
            .rsplit('/')
            .next()
            .unwrap_or("page.html")
            .to_owned(),
        inlined: files.inlined,
        left_out: files.left_out,
    })
}

/// Pass 1 (`lol_html`): our rewrites; returns the rewritten page (still to be sanitized).
fn rewrite<'h>(
    source: &str,
    prototype: bool,
    answers: Option<&'h PageAnswers>,
    files: &Rc<RefCell<Files<'h>>>,
    collected: &Rc<RefCell<Collected>>,
) -> Result<String, String> {
    let (c5, c6) = (collected.clone(), collected.clone());
    let (f2, f3, f4, f5) = (files.clone(), files.clone(), files.clone(), files.clone());
    let settings = collect_handlers(RewriteStrSettings::new(), collected, files)
        .append_element_content_handler(text!("script", move |t| {
            let mut c = c5.borrow_mut();
            if prototype && c.inert == 0 {
                c.script.push_str(t.as_str());
            }
            Ok(())
        }))
        .append_element_content_handler(element!("script", move |el| {
            let module = el
                .get_attribute("type")
                .is_some_and(|t| t.trim().eq_ignore_ascii_case("module"));
            let src = el.get_attribute("src");
            let inert = c6.borrow().inert > 0;
            if prototype && !inert {
                if let Some(src) = src {
                    let mut f = f2.borrow_mut();
                    let page = f.page.to_owned();
                    if let Some((_, text)) = f.read_text(&page, &src) {
                        c6.borrow_mut().scripts.push((text, module));
                    }
                } else if let Some(handlers) = el.end_tag_handlers() {
                    let inner = c6.clone();
                    let handler: lol_html::EndTagHandler<'static> = Box::new(move |_end| {
                        let mut c = inner.borrow_mut();
                        let text = std::mem::take(&mut c.script);
                        c.scripts.push((text, module));
                        Ok(())
                    });
                    handlers.push(handler);
                }
            }
            el.remove();
            Ok(())
        }))
        .append_element_content_handler(element!("[data-input][id]", move |el| {
            if let Some(id) = el.get_attribute("id") {
                el.after(&frozen(answers, id.trim()), ContentType::Html);
            }
            Ok(())
        }))
        .append_element_content_handler(element!(
            "img[src], video[src], video[poster], audio[src], source[src]",
            move |el| {
                for attr in ["src", "poster"] {
                    inline_attr(el, attr, &f3);
                }
                Ok(())
            }
        ))
        .append_element_content_handler(element!("a[href]", move |el| {
            let href = el.get_attribute("href").unwrap_or_default();
            let page = f4.borrow().page.to_owned();
            if let Target::Local { path, fragment } = resolve(&page, &href) {
                el.remove_attribute("href");
                // Named only when it is a real page of the handbook (an odd href is just dropped).
                let root = f4.borrow().root.to_path_buf();
                let file = path.split('/').fold(root.clone(), |p, s| p.join(s));
                if crate::paths::is_page_name(&path)
                    && file.is_file()
                    && crate::handbook::files::inside(&root, &file)
                {
                    let hint = format!(
                        "Handbook page {path}{} (not in this file)",
                        fragment.map(|f| format!("#{f}")).unwrap_or_default()
                    );
                    el.set_attribute("title", &hint)
                        .map_err(|e| e.to_string())?;
                }
            }
            Ok(())
        }))
        .append_element_content_handler(element!("[style]", move |el| {
            if let Some(style) = el.get_attribute("style") {
                let mut f = f5.borrow_mut();
                let page = f.page.to_owned();
                let css = f.css(&style, &page, 0);
                el.set_attribute("style", &css).map_err(|e| e.to_string())?;
            }
            Ok(())
        }));
    rewrite_str(source, settings).map_err(|e| format!("the page could not be rewritten: {e}"))
}

/// The handlers that collect the head's parts: the title, `<style>` and stylesheet text (inlined), and the open
/// inert containers (`<template>`, `<svg>`, `<noscript>`), whose scripts are never the page's scripts.
fn collect_handlers<'h>(
    settings: RewriteStrSettings<'h, 'static>,
    collected: &Rc<RefCell<Collected>>,
    files: &Rc<RefCell<Files<'h>>>,
) -> RewriteStrSettings<'h, 'static> {
    let (c1, c2, c3, c4) = (
        collected.clone(),
        collected.clone(),
        collected.clone(),
        collected.clone(),
    );
    let (f0, f1) = (files.clone(), files.clone());
    settings
        .append_element_content_handler(element!("template, svg, noscript", move |el| {
            if el.can_have_content() {
                c1.borrow_mut().inert += 1;
                let inner = c1.clone();
                if let Some(handlers) = el.end_tag_handlers() {
                    let handler: lol_html::EndTagHandler<'static> = Box::new(move |_end| {
                        let mut c = inner.borrow_mut();
                        c.inert = c.inert.saturating_sub(1);
                        Ok(())
                    });
                    handlers.push(handler);
                }
            }
            Ok(())
        }))
        .append_element_content_handler(text!("title", move |t| {
            c2.borrow_mut().title.push_str(t.as_str());
            Ok(())
        }))
        .append_element_content_handler(element!("title, head > meta, base", |el| {
            el.remove();
            Ok(())
        }))
        .append_element_content_handler(text!("style", move |t| {
            let mut c = c3.borrow_mut();
            c.style.push_str(t.as_str());
            if t.last_in_text_node() {
                // A `<style>` element's `url()`s and `@import`s are inlined relative to the page.
                let raw = std::mem::take(&mut c.style);
                let mut f = f0.borrow_mut();
                let page = f.page.to_owned();
                let css = f.css(&raw, &page, 0);
                c.css.push_str(&css);
                c.css.push('\n');
            }
            Ok(())
        }))
        .append_element_content_handler(element!("style", |el| {
            el.remove();
            Ok(())
        }))
        .append_element_content_handler(element!("link", move |el| {
            let rel = el
                .get_attribute("rel")
                .unwrap_or_default()
                .to_ascii_lowercase();
            if rel.split_whitespace().any(|r| r == "stylesheet")
                && let Some(href) = el.get_attribute("href")
            {
                let mut f = f1.borrow_mut();
                let page = f.page.to_owned();
                if let Some((path, text)) = f.read_text(&page, &href) {
                    let css = f.css(&text, &path, 0);
                    let mut c = c4.borrow_mut();
                    c.css.push_str(&css);
                    c.css.push('\n');
                }
            }
            el.remove();
            Ok(())
        }))
}

/// Replaces `attr` of `el` with a `data:` URL of the handbook file it names, or removes it.
fn inline_attr(el: &mut Element<'_, '_>, attr: &str, files: &Rc<RefCell<Files<'_>>>) {
    let Some(value) = el.get_attribute(attr) else {
        return;
    };
    let mut f = files.borrow_mut();
    let page = f.page.to_owned();
    match f.data_url(&page, &value) {
        Some(url) => {
            let _ = el.set_attribute(attr, &url);
        }
        None => el.remove_attribute(attr),
    }
}

/// The frozen answer shown after input `id` (§6.6 "exported pages MAY show the answers as text"), escaped as text.
fn frozen(answers: Option<&PageAnswers>, id: &str) -> String {
    let Some(item) = answers.and_then(|a| a.inputs.iter().find(|i| i.input.id == id)) else {
        return String::new();
    };
    let text = match (&item.answer, item.input.kind.as_str()) {
        (None, _) => "Not answered yet.".to_owned(),
        (Some(_), "rank") => format!(
            "Your order: {}",
            item.words
                .iter()
                .enumerate()
                .map(|(n, w)| format!("{}. {w}", n + 1))
                .collect::<Vec<_>>()
                .join(" · ")
        ),
        (Some(_), "checklist") if item.words.is_empty() => "Nothing ticked.".to_owned(),
        (Some(_), "checklist") => format!("Ticked: {}", item.words.join(" · ")),
        (Some(_), "text") => format!("Your answer: “{}”", item.words.join(" ")),
        (Some(_), _) => format!("Your choice: {}", item.words.join(" · ")),
    };
    format!("<p class=\"hb-answer\">{}</p>", escape(&text))
}
