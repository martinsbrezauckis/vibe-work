//! `WKWebView` printing uses `WebKit`'s paginated print layout and link drawing. `createPDF` captures a rectangle
//! as a single page, so the print operation is used instead. Called by the native preview on the UI thread.
//! The paper size follows the locale's region (`regionCode` on macOS 13+, the deprecated `countryCode` on 11-12,
//! which have no `regionCode`) and the job is saved to a file URL (`NSPrintJobSavingURL`, not the deprecated
//! `NSPrintSavePath`).
//!
//! The print runs on its own thread (`runOperationModalForWindow:` with `canSpawnSeparateThread`), never on the main
//! thread: `WebKit` learns the page count from the page's process through the main thread, so a print that holds the
//! main thread (`runOperation`) never gets it and writes empty pages without end (measured: hundreds of MB in
//! seconds, the whole app frozen). With no print or progress panel nothing is shown over the window.
//!
//! Depends on: objc2 (`define_class!` for the completion delegate), objc2-app-kit (`NSPrintInfo`,
//! `NSPrintOperation`), objc2-web-kit (`WKWebView`, `WKPreferences`). Called by: vw-webview `pdf::print`.
//! Invariants: `print_pdf` answers `done` exactly once (from the print thread when the print ends, or at once on a
//! failure before it starts); the preview's own background-printing setting is put back on the main thread after
//! the print.
use super::{PdfFile, PngDone};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool, NSObject, ProtocolObject};
use objc2::{AnyThread, DefinedClass, define_class, msg_send, sel};
use objc2_app_kit::{
    NSPrintInfo, NSPrintJobSavingURL, NSPrintOperation, NSPrintSaveJob, NSPrintingPaginationMode,
};
use objc2_foundation::{NSLocale, NSObjectProtocol, NSSize, NSString, NSURL};
use objc2_web_kit::{WKPreferences, WKWebView};
use std::cell::Cell;
use std::ffi::c_void;

/// What the completion delegate needs when the print ends.
struct PrintEnd {
    /// The caller's callback and the staging file; taken once, on the print thread.
    finish: Cell<Option<(PngDone, PdfFile)>>,
    /// The preview's preferences and its own background-printing value; put back on the main thread.
    restore: Cell<Option<(Retained<WKPreferences>, bool)>>,
}

define_class!(
    /// The print operation's delegate: hands the finished PDF to the caller, then puts the preview's setting back.
    #[unsafe(super(NSObject))]
    #[name = "VibeWorkPdfPrintEnd"]
    #[ivars = PrintEnd]
    struct PrintDelegate;

    impl PrintDelegate {
        /// Called once by `AppKit` on the print thread when the print ended; `context` holds the reference that
        /// keeps this delegate alive until then (the operation does not retain its delegate).
        #[unsafe(method(printOperationDidRun:success:contextInfo:))]
        fn did_run(&self, _operation: &NSPrintOperation, success: Bool, context: *mut c_void) {
            if let Some((done, staged)) = self.ivars().finish.take() {
                let result = if success.as_bool() {
                    staged.read()
                } else {
                    Err("the browser could not print the PDF".into())
                };
                // The staging folder is removed here, before the caller sees the result.
                drop(staged);
                done(result);
            }
            // SAFETY: `performSelectorOnMainThread:withObject:waitUntilDone:` retains this object until the selector
            // ran; `context` is the +1 reference `print_pdf` handed over, released here exactly once.
            unsafe {
                let _: () = msg_send![
                    self,
                    performSelectorOnMainThread: sel!(restoreBackgrounds),
                    withObject: None::<&AnyObject>,
                    waitUntilDone: Bool::NO
                ];
                drop(Retained::from_raw(context.cast::<Self>()));
            }
        }

        /// Puts the preview's background-printing setting back (main thread: `WKPreferences` is main-thread only).
        #[unsafe(method(restoreBackgrounds))]
        fn restore_backgrounds(&self) {
            if let Some((preferences, previous)) = self.ivars().restore.take() {
                // SAFETY: a plain property write on the main thread (this selector runs there).
                unsafe { preferences.setShouldPrintBackgrounds(previous) };
            }
        }
    }
);

/// Prints the mounted document without a dialog and answers `done` with the completed PDF.
pub fn print_pdf(view: *mut std::ffi::c_void, done: PngDone) {
    // SAFETY: Tauri supplies a live WKWebView on the UI thread; it stays alive for this call.
    let Some(view) = (unsafe { view.cast::<WKWebView>().as_ref() }) else {
        done(Err("the webview closed".into()));
        return;
    };
    let Some(window) = view.window() else {
        done(Err("the page is not in a window".into()));
        return;
    };
    let staged = match PdfFile::new() {
        Ok(staged) => staged,
        Err(e) => {
            done(Err(e));
            return;
        }
    };
    let info = NSPrintInfo::init(NSPrintInfo::alloc());
    let locale = NSLocale::currentLocale();
    let region = if locale.respondsToSelector(sel!(regionCode)) {
        locale.regionCode()
    } else {
        #[allow(deprecated)]
        locale.countryCode()
    };
    let country = region.map(|s| s.to_string()).unwrap_or_default();
    let letter = matches!(country.as_str(), "US" | "CA");
    info.setPaperSize(if letter {
        NSSize::new(612.0, 792.0)
    } else {
        NSSize::new(595.276, 841.89)
    });
    info.setHorizontalPagination(NSPrintingPaginationMode::Fit);
    info.setVerticalPagination(NSPrintingPaginationMode::Automatic);
    info.setVerticallyCentered(false);
    // SAFETY: `NSPrintSaveJob` and `NSPrintJobSavingURL` are AppKit constants, valid for the process's life; the
    // dictionary takes a file URL for that key.
    unsafe {
        info.setJobDisposition(NSPrintSaveJob);
        let path = NSString::from_str(&staged.path().to_string_lossy());
        let url = NSURL::fileURLWithPath(&path);
        info.dictionary()
            .setObject_forKey(url.as_ref(), ProtocolObject::from_ref(NSPrintJobSavingURL));
    }
    // SAFETY: property reads and writes of the live webview on the main thread; the operation takes `info`.
    let (preferences, previous, operation) = unsafe {
        let preferences = view.configuration().preferences();
        let previous = preferences.shouldPrintBackgrounds();
        preferences.setShouldPrintBackgrounds(true);
        let operation = view.printOperationWithPrintInfo(&info);
        (preferences, previous, operation)
    };
    operation.setShowsPrintPanel(false);
    operation.setShowsProgressPanel(false);
    operation.setCanSpawnSeparateThread(true);
    let delegate = PrintDelegate::alloc().set_ivars(PrintEnd {
        finish: Cell::new(Some((done, staged))),
        restore: Cell::new(Some((preferences, previous))),
    });
    // SAFETY: `init` on a freshly allocated NSObject subclass.
    let delegate: Retained<PrintDelegate> = unsafe { msg_send![super(delegate), init] };
    let context = Retained::into_raw(delegate.clone()).cast::<c_void>();
    let target: &AnyObject = &delegate;
    // SAFETY: the delegate implements `printOperationDidRun:success:contextInfo:` with the signature AppKit calls;
    // `context` is a +1 reference to it that `did_run` releases. The call returns at once; the print goes on on
    // its own thread and ends in `did_run`.
    unsafe {
        operation.runOperationModalForWindow_delegate_didRunSelector_contextInfo(
            &window,
            Some(target),
            Some(sel!(printOperationDidRun:success:contextInfo:)),
            context,
        );
    }
}
