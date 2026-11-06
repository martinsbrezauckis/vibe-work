//! GTK `AppIndicator` ownership, including its native `NeedsAttention` status. Called on the main
//! thread by the desktop shell; keeps the existing Open/Quit menu and uses no polling for attention.
use gtk::prelude::*;
use libappindicator::{AppIndicator, AppIndicatorStatus};
use std::{cell::RefCell, path::Path};

thread_local! {
    static ICON: RefCell<Option<AppIndicator>> = const { RefCell::new(None) };
}

/// Creates the indicator on GTK's main thread with normal and attention artwork.
/// # Errors
/// Artwork could not be saved in the app's own directory.
pub fn create(
    directory: &Path,
    normal: &[u8],
    attention: (&[u8], u32, u32),
    open_label: &str,
    quit_label: &str,
    open: impl Fn() + 'static,
    quit: impl Fn() + 'static,
) -> Result<(), String> {
    crate::ensure_private_dir(directory).map_err(|error| error.to_string())?;
    let ordinary = directory.join("normal.png");
    let marked = directory.join("attention.png");
    std::fs::write(&ordinary, normal).map_err(|error| error.to_string())?;
    let output = std::fs::File::create(&marked).map_err(|error| error.to_string())?;
    let mut encoder = png::Encoder::new(output, attention.1, attention.2);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .and_then(|mut writer| writer.write_image_data(attention.0))
        .map_err(|error| error.to_string())?;
    let mut menu = gtk::Menu::new();
    let open_item = gtk::MenuItem::with_label(open_label);
    open_item.connect_activate(move |_| open());
    let quit_item = gtk::MenuItem::with_label(quit_label);
    quit_item.connect_activate(move |_| quit());
    menu.append(&open_item);
    menu.append(&gtk::SeparatorMenuItem::new());
    menu.append(&quit_item);
    menu.show_all();
    let mut icon = AppIndicator::new("vibe-work", &ordinary.to_string_lossy());
    icon.set_attention_icon_full(&marked.to_string_lossy(), "Needs your answer");
    icon.set_menu(&mut menu);
    icon.set_status(AppIndicatorStatus::Active);
    ICON.with(|slot| *slot.borrow_mut() = Some(icon));
    Ok(())
}

/// Updates the native `StatusNotifier` state; a missing tray leaves the window's behaviour intact.
pub fn set_attention(waiting: bool) {
    ICON.with(|slot| {
        if let Some(icon) = slot.borrow_mut().as_mut() {
            icon.set_status(if waiting {
                AppIndicatorStatus::Attention
            } else {
                AppIndicatorStatus::Active
            });
        }
    });
}
