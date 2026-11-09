//! Bounded URL downloads shared by all native engines, using the selected webview's cookie manager internally.
//! No cookie values enter tool receipts. Each redirect gets only its own cookies; native click downloads use Wry.
use std::path::PathBuf;
use std::time::Duration;
use tauri::{Webview, Wry};
use tokio::io::AsyncWriteExt;
use url::Url;
use vw_preview::EngineError;

pub(super) async fn run(
    webview: Webview<Wry>,
    url: Url,
    destination: PathBuf,
    max: u64,
    proxy: Option<String>,
) -> Result<(), EngineError> {
    let result = tokio::time::timeout(
        Duration::from_secs(300),
        transfer(webview, url, destination, max, proxy),
    )
    .await;
    result
        .map_err(|_| EngineError::Timeout("downloading the URL".into()))?
        .map_err(EngineError::Platform)
}

async fn transfer(
    webview: Webview<Wry>,
    mut url: Url,
    destination: PathBuf,
    max: u64,
    proxy: Option<String>,
) -> Result<(), String> {
    let mut builder = vw_mistral::tls_builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10));
    if let Some(proxy) = proxy {
        builder = builder
            .proxy(reqwest::Proxy::all(proxy).map_err(|_| "Download network route unavailable.")?);
    }
    let client = builder
        .build()
        .map_err(|_| "Download client unavailable.")?;
    for _ in 0..=10 {
        if !matches!(url.scheme(), "http" | "https")
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err("Download redirect is not an ordinary HTTP(S) URL.".into());
        }
        let view = webview.clone();
        let address = url.clone();
        let cookies = tokio::task::spawn_blocking(move || view.cookies_for_url(address))
            .await
            .map_err(|_| "Cookie store unavailable.")?
            .map_err(|_| "Cookie store unavailable.")?;
        let header = cookies
            .iter()
            .map(|c| format!("{}={}", c.name(), c.value()))
            .collect::<Vec<_>>()
            .join("; ");
        let mut request = client.get(url.clone());
        if !header.is_empty() {
            request = request.header(reqwest::header::COOKIE, header);
        }
        let mut response = request
            .send()
            .await
            .map_err(|_| "Download request failed.")?;
        if response.status().is_redirection() {
            url = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| url.join(v).ok())
                .ok_or("Download redirect has no usable URL.")?;
            continue;
        }
        if !response.status().is_success() {
            return Err(format!(
                "Download server answered {}.",
                response.status().as_u16()
            ));
        }
        if response.content_length().is_some_and(|n| n > max) {
            return Err("Download is too large.".into());
        }
        let expected = response.content_length();
        let mut file = tokio::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(destination)
            .await
            .map_err(|_| "Download staging file could not be created.")?;
        let mut bytes = 0u64;
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| "Download was interrupted.")?
        {
            bytes = bytes.saturating_add(chunk.len() as u64);
            if bytes > max {
                return Err("Download is too large.".into());
            }
            file.write_all(&chunk)
                .await
                .map_err(|_| "Download could not be written.")?;
        }
        if expected.is_some_and(|n| n != bytes) {
            return Err("Download ended before all bytes arrived.".into());
        }
        file.flush()
            .await
            .map_err(|_| "Download could not be saved.")?;
        return Ok(());
    }
    Err("Download followed too many redirects.".into())
}

impl super::TauriEngine {
    pub(super) fn remember_download_proxy(&self, label: &str, proxy: Option<&str>) {
        if let Some(proxy) = proxy {
            self.download_proxies
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(label.to_owned(), proxy.to_owned());
        }
    }
}
