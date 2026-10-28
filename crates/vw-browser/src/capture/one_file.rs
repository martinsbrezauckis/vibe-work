//! Produces a one-file page capture without sidecars. Depends on the authorized live mount, native printing,
//! passive DOM reader and resource packer; called by the capture runner before its atomic file commit.
use super::{Fetch, PageFormat, page, single_html};
use crate::{driver::Browser, scripts};

pub(super) async fn produce(
    browser: &Browser,
    session: &str,
    format: PageFormat,
    fetch: &dyn Fetch,
    url: &str,
    captured_at: &str,
) -> Result<(Vec<u8>, usize), String> {
    let page_id = browser.page_of(session).await?;
    let mount = browser
        .service()
        .page_mount(session)
        .ok_or("the page closed before capture")?;
    let result = match format {
        PageFormat::Pdf => {
            #[cfg(target_os = "linux")]
            let links: super::pdf_links::Links = browser
                .run_script(session, scripts::pdf_links(&page_id))
                .await?;
            #[cfg(target_os = "linux")]
            browser.check_answer(session, &page_id, &links.url)?;
            let bytes = browser
                .service()
                .print_pdf_on_mount(&mount)
                .await
                .map_err(|e| e.to_string())?;
            if !bytes.starts_with(b"%PDF-") {
                return Err("the engine did not produce a PDF".into());
            }
            #[cfg(target_os = "linux")]
            let bytes = if links.links.is_empty() {
                bytes
            } else {
                tokio::task::spawn_blocking(move || {
                    super::pdf_links::annotate(&bytes, &links.links)
                })
                .await
                .map_err(|e| e.to_string())??
            };
            (bytes, 0)
        }
        PageFormat::SingleHtml => {
            let (text, cut) = page::read(browser, session, page::Format::SingleHtml).await?;
            if cut {
                return Err("the page exceeds the 50 MB one-file limit".into());
            }
            let doc = serde_json::from_str(&text)
                .map_err(|_| "the page did not return a complete document")?;
            let (html, kept) =
                single_html::pack(doc, session, fetch, single_html::MAX_TOTAL - 4096).await?;
            let html = page::file_text(page::Format::SingleHtml, &html, url, captured_at, false);
            (html.into_bytes(), kept)
        }
        PageFormat::Text(format) => {
            let (text, cut) = page::read(browser, session, format).await?;
            (
                page::file_text(format, &text, url, captured_at, cut).into_bytes(),
                0,
            )
        }
        PageFormat::Png => (super::shot::full_page(browser, session, None).await?.png, 0),
    };
    // Fetching and printing can outlive a tab close or a navigation. Recheck the original document before writing.
    if browser.service().page_mount(session).as_ref() != Some(&mount) {
        return Err("the page closed during capture".into());
    }
    let answer: serde_json::Value = browser
        .run_in_mount(
            session,
            scripts::fullpage(&page_id, "metrics", 0.0),
            Some(&mount),
        )
        .await?;
    browser.check_answer(
        session,
        &page_id,
        answer["url"].as_str().unwrap_or_default(),
    )?;
    Ok(result)
}
