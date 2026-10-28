//! Shows one `vw_platform::confirm_window` confirmation in a small app of its own (macOS: an `NSApplication` with one
//! plain window as the sheet's parent; Windows: a task dialog without a parent) and prints how it ended, so the
//! accessibility probes (macOS Accessibility, Windows UI Automation) can answer every button
//! layout the app uses: `Don't allow` / `Allow once` / `Always …`, `Keep` / `Close`, `Cancel` / `Continue`.
//!
//! Usage: confirm_window_probe [--close-after <seconds>] <question> <text below> <button>...   (buttons in the shown
//! order, the first = the declining default; 1 to 3). Prints `SHOWN` once the window is up, then
//! `ANSWER <index>` or `ANSWER none` (declined without a button, or closed by --close-after), and exits.
//! Windows: the task dialog needs common controls v6, which the app gets from its manifest; build this example with
//! an application manifest that selects Microsoft.Windows.Common-Controls 6.0 embedded, or it does not start.
#![allow(
    unsafe_code,
    reason = "the probe's own app window on macOS (AppKit window creation); the module under test keeps its own"
)]

#[cfg(any(target_os = "macos", windows))]
fn args() -> (Option<f64>, vw_platform::confirm_window::ConfirmSpec) {
    let mut all: Vec<String> = std::env::args().skip(1).collect();
    let mut close_after = None;
    if all.first().map(String::as_str) == Some("--close-after") && all.len() > 1 {
        close_after = all[1].parse().ok();
        all.drain(..2);
    }
    let mut parts = all.into_iter();
    let heading = parts.next().unwrap_or_default();
    let body = parts.next().unwrap_or_default();
    let spec = vw_platform::confirm_window::ConfirmSpec {
        title: "Vibe Work asks for your confirmation".to_owned(),
        heading,
        body,
        buttons: parts.collect(),
    };
    (close_after, spec)
}

#[cfg(any(target_os = "macos", windows))]
fn report(answer: Option<usize>) {
    match answer {
        Some(index) => println!("ANSWER {index}"),
        None => println!("ANSWER none"),
    }
}

#[cfg(target_os = "macos")]
fn main() {
    use objc2::rc::Retained;
    use objc2::{MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{
        NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSWindow,
        NSWindowStyleMask,
    };
    use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

    let (close_after, spec) = args();
    let Some(mtm) = MainThreadMarker::new() else {
        eprintln!("not on the main thread");
        std::process::exit(2);
    };
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
    let frame = NSRect::new(NSPoint::new(200.0, 200.0), NSSize::new(640.0, 420.0));
    // SAFETY: a plain titled window created on the main thread; released explicitly at exit (process end).
    let window: Retained<NSWindow> = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            frame,
            NSWindowStyleMask::Titled | NSWindowStyleMask::Closable,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    // SAFETY: closing must not free the window under the Retained we keep.
    unsafe { window.setReleasedWhenClosed(false) };
    window.setTitle(&NSString::from_str("confirm_window_probe"));
    window.makeKeyAndOrderFront(None);
    #[allow(
        deprecated,
        reason = "the probe brings itself forward like a launched app"
    )]
    app.activateIgnoringOtherApps(true);
    let parent = Retained::as_ptr(&window)
        .cast_mut()
        .cast::<std::ffi::c_void>();
    let reply = Box::new(|answer: Option<usize>| {
        report(answer);
        std::process::exit(0);
    });
    match vw_platform::confirm_window::show(parent, &spec, reply) {
        Ok(id) => {
            println!("SHOWN pid={}", std::process::id());
            if let Some(seconds) = close_after {
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs_f64(seconds));
                    dispatch2::DispatchQueue::main()
                        .exec_async(move || vw_platform::confirm_window::close(id));
                });
            }
        }
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    }
    app.run();
}

#[cfg(windows)]
fn main() {
    let (close_after, spec) = args();
    let (send, receive) = std::sync::mpsc::channel();
    let reply = Box::new(move |answer: Option<usize>| {
        let _ = send.send(answer);
    });
    let open = match vw_platform::confirm_window::show(0, &spec, reply) {
        Ok(open) => open,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    };
    println!("SHOWN pid={}", std::process::id());
    let answer = if let Some(seconds) = close_after {
        if let Ok(answer) = receive.recv_timeout(std::time::Duration::from_secs_f64(seconds)) {
            answer
        } else {
            // The deadline of this probe: close the window from outside, then take its answer (none).
            drop(open);
            receive.recv().ok().flatten()
        }
    } else {
        receive.recv().ok().flatten()
    };
    report(answer);
}

#[cfg(not(any(target_os = "macos", windows)))]
fn main() {
    eprintln!("confirm_window_probe is for macOS and Windows (Linux uses the app's GTK window)");
    std::process::exit(2);
}
