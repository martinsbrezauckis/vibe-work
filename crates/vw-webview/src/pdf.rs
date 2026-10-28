//! Silent, paginated PDF output from the mounted page. Native output stays in private staging until complete;
//! the browser capture owns the final single-file commit. Depends on native engine APIs; called by `TauriEngine`.
use std::time::Duration;
use tauri::{Webview, Wry};
use vw_preview::EngineError;

pub(crate) async fn print(webview: &Webview<Wry>) -> Result<Vec<u8>, EngineError> {
    crate::platform::with_native(
        webview,
        "saving the PDF",
        Duration::from_secs(35),
        |native, reply| {
            #[cfg(target_os = "linux")]
            {
                use std::cell::RefCell;
                use std::rc::Rc;
                use webkit2gtk::{PrintOperationExt as _, SettingsExt as _};
                let staged = match vw_platform::webview::PdfFile::new() {
                    Ok(staged) => staged,
                    Err(e) => {
                        let _ = reply.send(Err(e));
                        return;
                    }
                };
                let Ok(uri) = url::Url::from_file_path(staged.path()) else {
                    let _ = reply.send(Err("invalid PDF staging path".into()));
                    return;
                };
                let locale = ["LC_ALL", "LC_PAPER", "LANG"]
                    .iter()
                    .find_map(|key| std::env::var(key).ok().filter(|s| !s.is_empty()))
                    .unwrap_or_default();
                let letter = locale
                    .split(['_', '-', '.'])
                    .any(|s| matches!(s, "US" | "CA"));
                let setup = gtk::PageSetup::new();
                setup.set_paper_size(&gtk::PaperSize::new(Some(if letter {
                    "na_letter"
                } else {
                    "iso_a4"
                })));
                setup.set_orientation(gtk::PageOrientation::Portrait);
                let settings = gtk::PrintSettings::new();
                settings.set_printer("Print to File");
                settings.set("output-file-format", Some("pdf"));
                settings.set("output-uri", Some(uri.as_str()));
                settings.set_print_pages(gtk::PrintPages::All);
                let view = native.inner();
                let background = webkit2gtk::WebViewExt::settings(&view).map(|s| {
                    let old = s.is_print_backgrounds();
                    s.set_print_backgrounds(true);
                    (s, old)
                });
                let operation = webkit2gtk::PrintOperation::new(&view);
                operation.set_page_setup(&setup);
                operation.set_print_settings(&settings);
                let failure = Rc::new(RefCell::new(None));
                let failed = failure.clone();
                operation.connect_failed(move |_, error| {
                    *failed.borrow_mut() = Some(error.to_string());
                });
                let completion = Rc::new(RefCell::new(Some((reply, staged, background))));
                operation.connect_finished(move |_| {
                    if let Some((reply, staged, background)) = completion.borrow_mut().take() {
                        if let Some((settings, old)) = background {
                            settings.set_print_backgrounds(old);
                        }
                        let result = failure
                            .borrow_mut()
                            .take()
                            .map_or_else(|| staged.read(), Err);
                        let _ = reply.send(result);
                    }
                });
                operation.print();
            }
            #[cfg(windows)]
            vw_platform::webview::print_pdf(
                &native.controller(),
                &native.environment(),
                Box::new(move |result| {
                    let _ = reply.send(result);
                }),
            );
            #[cfg(target_os = "macos")]
            vw_platform::webview::print_pdf(
                native.inner(),
                Box::new(move |result| {
                    let _ = reply.send(result);
                }),
            );
        },
    )
    .await
}
