//! `WebView2`'s asynchronous `PrintToPdf` API (the COM counterpart of `PrintToPdfAsync`).
//! Called on the UI thread; a completion handler owns private staging through cancellation.
use super::{PdfFile, PngDone};
use std::sync::{Arc, Mutex, PoisonError};
use webview2_com::{
    Microsoft::Web::WebView2::Win32::{
        ICoreWebView2_7, ICoreWebView2Controller, ICoreWebView2Environment,
        ICoreWebView2Environment6,
    },
    PrintToPdfCompletedHandler,
};
use windows::core::{HSTRING, Interface};

/// Prints the mounted document without a dialog and returns the completed PDF.
pub fn print_pdf(
    controller: &ICoreWebView2Controller,
    environment: &ICoreWebView2Environment,
    done: PngDone,
) {
    let completion = Arc::new(Mutex::new(Some(done)));
    let finish = completion.clone();
    // SAFETY: all COM objects and the handler are used on their owning UI thread. WebView2 retains the
    // completion handler until output has finished; private staging is moved into that handler.
    let result = unsafe {
        (|| -> Result<(), String> {
            let staged = PdfFile::new()?;
            let view: ICoreWebView2_7 = controller
                .CoreWebView2()
                .and_then(|v| v.cast())
                .map_err(|e| e.to_string())?;
            let env: ICoreWebView2Environment6 = environment.cast().map_err(|e| e.to_string())?;
            let settings = env.CreatePrintSettings().map_err(|e| e.to_string())?;
            let mut locale = [0u16; 85];
            let len = windows::Win32::Globalization::GetUserDefaultLocaleName(&mut locale);
            let locale = String::from_utf16_lossy(
                &locale[..usize::try_from(len.saturating_sub(1)).unwrap_or(0)],
            );
            let letter = locale.split('-').any(|part| matches!(part, "US" | "CA"));
            settings
                .SetPageWidth(if letter { 8.5 } else { 210.0 / 25.4 })
                .map_err(|e| e.to_string())?;
            settings
                .SetPageHeight(if letter { 11.0 } else { 297.0 / 25.4 })
                .map_err(|e| e.to_string())?;
            settings
                .SetShouldPrintBackgrounds(true)
                .map_err(|e| e.to_string())?;
            settings
                .SetShouldPrintHeaderAndFooter(false)
                .map_err(|e| e.to_string())?;
            let path = HSTRING::from(staged.path().as_os_str());
            let handler = PrintToPdfCompletedHandler::create(Box::new(move |status, success| {
                let result = status.map_err(|e| e.to_string()).and_then(|()| {
                    if success {
                        staged.read()
                    } else {
                        Err("the browser could not print the PDF".into())
                    }
                });
                if let Some(done) = finish.lock().unwrap_or_else(PoisonError::into_inner).take() {
                    done(result);
                }
                Ok(())
            }));
            view.PrintToPdf(&path, &settings, &handler)
                .map_err(|e| e.to_string())
        })()
    };
    if let Err(error) = result
        && let Some(done) = completion
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    {
        done(Err(error));
    }
}
