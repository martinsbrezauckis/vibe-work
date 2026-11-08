//! GTK `AppIndicator` ownership, including its native `NeedsAttention` status. Called on the main
//! thread by the desktop shell; keeps the existing Open/Quit menu and uses no polling for attention.
use gtk::prelude::*;
use libappindicator::{AppIndicator, AppIndicatorStatus};
use std::{cell::RefCell, path::Path};

thread_local! {
    static ICON: RefCell<Option<AppIndicator>> = const { RefCell::new(None) };
}

/// One app-owned native menu action; the final action follows a separator.
pub type MenuAction<'a> = (&'a str, Box<dyn Fn()>);

/// Creates the indicator on GTK's main thread with normal and attention artwork.
/// # Errors
/// Artwork could not be saved in the app's own directory.
pub fn create(
    directory: &Path,
    normal: &[u8],
    attention: (&[u8], u32, u32),
    actions: Vec<MenuAction<'_>>,
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
    let last = actions.len().saturating_sub(1);
    for (index, (label, action)) in actions.into_iter().enumerate() {
        if index == last && index > 0 {
            menu.append(&gtk::SeparatorMenuItem::new());
        }
        let item = gtk::MenuItem::with_label(label);
        item.connect_activate(move |_| action());
        menu.append(&item);
    }
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
