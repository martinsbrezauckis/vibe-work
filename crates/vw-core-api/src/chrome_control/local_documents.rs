//! Confined single-file previews for Firefox: immutable bytes, passive HTTP, bounded lifetime and truthful receipts.
//! Network admission calls prepare after shared authorization; Stop/switch/removal revoke the held servers.
use super::agent_browser::Browser;
use crate::core::Shared;
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    sync::{Arc, PoisonError},
};
#[derive(Default)]
pub(super) struct Documents(VecDeque<Entry>);
struct Entry {
    chat: String,
    url: String,
    source: String,
    _server: vw_pages::PagesServer,
}
impl Documents {
    pub fn clear(&mut self) {
        self.0.clear();
    }
    pub fn remove(&mut self, chat: &str) {
        self.0.retain(|e| e.chat != chat);
    }
    pub fn annotate(&self, chat: &str, result: &mut Value) {
        if result["structuredContent"]["status"] != "completed" {
            return;
        }
        let url = result["structuredContent"]["data"]["tab"]["url"]
            .as_str()
            .unwrap_or_default();
        if let Some(entry) = self.0.iter().find(|e| e.chat == chat && e.url == url) {
            result["structuredContent"]["data"]["localDocument"] = json!({"source":entry.source,"view":"passive snapshot","instruction":"Scripts, frames, forms and relative assets are disabled. For an interactive or multi-file page, use an existing local web server. Reopen the file URL to refresh the snapshot."});
        }
    }
}
pub(super) async fn prepare(
    shared: &Arc<Shared>,
    chat: &str,
    args: &mut Value,
) -> Result<(), String> {
    let Some(text) = args["args"]["url"]
        .as_str()
        .filter(|u| u.starts_with("file:"))
    else {
        return Ok(());
    };
    if !matches!(
        args["action"].as_str(),
        Some("page.navigate" | "tabs.create")
    ) {
        return Err("Firefox cannot use a local file URL for this action. Open a PDF, HTML or image with page.navigate or tabs.create inside the chat folder.".into());
    }
    let chrome = &shared.chrome_control;
    if !Browser::Firefox.enabled(shared) {
        return Err(Browser::Firefox.words(super::browser_text::SWITCHED_OFF));
    }
    let generation = chrome.browser_generation(Browser::Firefox);
    let stop = crate::stops::generation(shared, chat);
    let (source, file) = crate::browser_local::load(shared, chat, text).await?;
    let (server,url)=vw_pages::PagesServer::start_document(&file.name,file.data).await.map_err(|e|format!("The local document preview could not start: {e} Choose a smaller PDF, HTML or image in this chat folder."))?;
    let mut url = url::Url::parse(&url)
        .map_err(|_| "The preview address is invalid. Open the file again.")?;
    url.set_fragment(source.fragment());
    let url = url.to_string();
    let mut documents = chrome
        .local_documents
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if generation != chrome.browser_generation(Browser::Firefox)
        || stop != crate::stops::generation(shared, chat)
    {
        return Err("The browser work stopped while loading the file. Start a new browser session before opening it again.".into());
    }
    while documents.0.len() >= 8 {
        documents.0.pop_front();
    }
    documents.0.push_back(Entry {
        chat: chat.into(),
        url: url.clone(),
        source: source.to_string(),
        _server: server,
    });
    args["args"]["url"] = json!(url);
    Ok(())
}
