//! macOS Dock icon with the waiting count drawn into it (`vw_host::attention::dock_icon` supplies the pixels).
//!
//! `NSDockTile` badge labels follow the app's notification settings: macOS hides them until the person allows
//! the app's notifications, and the first notification the app posts marks it as not allowed until they answer
//! the system banner. The application icon image is drawn whatever those settings are, so the count lives there.

use objc2::{AnyThread, MainThreadMarker};
use objc2_app_kit::{NSApplication, NSImage};
use objc2_foundation::NSData;

/// Shows `rgba` (`width` x `height`) as the app's Dock icon, or the bundle icon again for `None`. Main thread only:
/// on another thread nothing changes and an error is returned.
///
/// # Errors
/// Not on the main thread, or the pixels could not be turned into an image.
pub fn show(icon: Option<(&[u8], u32, u32)>) -> Result<(), &'static str> {
    let main = MainThreadMarker::new().ok_or("the Dock icon changes on the main thread")?;
    let app = NSApplication::sharedApplication(main);
    let Some((rgba, width, height)) = icon else {
        // SAFETY: nil restores the icon from the bundle (documented AppKit behaviour); main thread checked above.
        unsafe { app.setApplicationIconImage(None) };
        return Ok(());
    };
    let pixels = image::RgbaImage::from_raw(width, height, rgba.to_vec())
        .ok_or("Dock icon size mismatch")?;
    let mut png = std::io::Cursor::new(Vec::new());
    pixels
        .write_to(&mut png, image::ImageFormat::Png)
        .map_err(|_| "Dock icon encoding failed")?;
    let data = NSData::with_bytes(png.get_ref());
    let image =
        NSImage::initWithData(NSImage::alloc(), &data).ok_or("Dock icon image unreadable")?;
    // SAFETY: a valid image on the main thread (checked above).
    unsafe { app.setApplicationIconImage(Some(&image)) };
    Ok(())
}
