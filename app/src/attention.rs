//! Thin native attention wiring. Artwork/count policy lives in vw-host; Linux indicator ownership in vw-platform.
use tauri::AppHandle;

/// Updates the app badge and tray from one count supplied by the session state controller.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub fn desktop_attention(app: AppHandle, count: u32) -> tauri::Result<()> {
    #[cfg(target_os = "linux")]
    {
        app.run_on_main_thread(move || vw_platform::tray::indicator::set_attention(count > 0))?;
    }
    #[cfg(target_os = "macos")]
    if let Some(window) = crate::main_window(&app) {
        window.set_badge_count((count > 0).then_some(i64::from(count)))?;
    }
    #[cfg(windows)]
    if let Some(window) = crate::main_window(&app) {
        let size = tauri::LogicalSize::new(
            vw_host::attention::BADGE_SIZE,
            vw_host::attention::BADGE_SIZE,
        )
        .to_physical::<u32>(window.scale_factor()?)
        .width
        .clamp(16, 128);
        window.set_overlay_icon(
            vw_host::attention::badge_at_size(count, size)
                .map(|rgba| tauri::image::Image::new_owned(rgba, size, size)),
        )?;
    }
    #[cfg(not(target_os = "linux"))]
    if let Some(tray) = app.tray_by_id("main") {
        #[cfg(windows)]
        let normal = tauri::include_image!("icons/tray-windows.ico");
        #[cfg(target_os = "macos")]
        let normal = tauri::include_image!("icons/tray-template@2x.png");
        let marked = vw_host::attention::tray(
            normal.rgba(),
            normal.width(),
            normal.height(),
            cfg!(target_os = "macos"),
            count > 0,
        );
        tray.set_icon_with_as_template(
            Some(tauri::image::Image::new_owned(
                marked,
                normal.width(),
                normal.height(),
            )),
            cfg!(target_os = "macos"),
        )?;
        tray.set_tooltip(Some(vw_host::attention::description(count)))?;
    }
    Ok(())
}
