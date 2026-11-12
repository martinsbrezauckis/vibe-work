//! Small native attention artwork. Depends only on RGBA pixels; desktop wiring supplies the normal tray icon.
//! Counts are rendered as text, never notification totals; the clear state restores the original image exactly.

/// Windows taskbar overlay size at 96 dpi (`ITaskbarList3::SetOverlayIcon`).
pub const BADGE_SIZE: u32 = 16;

/// A count for the constrained Windows overlay.
#[must_use]
pub fn label(count: u32) -> String {
    if count > 99 {
        "99+".into()
    } else {
        count.to_string()
    }
}

/// Plain state for native tray accessibility/tooltip text.
#[must_use]
pub fn description(count: u32) -> String {
    match count {
        0 => "Vibe Work".into(),
        1 => "Vibe Work: 1 chat needs your answer".into(),
        n => format!("Vibe Work: {n} chats need your answer"),
    }
}

/// Numeric taskbar overlay, or no overlay when nothing waits.
#[must_use]
pub fn badge(count: u32) -> Option<Vec<u8>> {
    if count == 0 {
        return None;
    }
    let mut pixels = vec![0; 16 * 16 * 4];
    // A rounded square leaves enough width for two full-size digits at the actual taskbar size.
    for y in 0_usize..16 {
        for x in 0_usize..16 {
            if (2..14).contains(&x) || (2..14).contains(&y) {
                let at = (y * 16 + x) * 4;
                pixels[at..at + 4].copy_from_slice(&[250, 80, 15, 255]);
            }
        }
    }
    let text = label(count);
    let wide = if text.len() < 3 { 2 } else { 1 };
    let advance = 4 * wide;
    let left = (16 - (text.len() * advance - wide)) / 2;
    for (index, digit) in text.chars().enumerate() {
        let rows = match digit {
            '0' => [7, 5, 5, 5, 7],
            '1' => [2, 6, 2, 2, 7],
            '2' => [7, 1, 7, 4, 7],
            '3' => [7, 1, 7, 1, 7],
            '4' => [5, 5, 7, 1, 1],
            '5' => [7, 4, 7, 1, 7],
            '6' => [7, 4, 7, 5, 7],
            '7' => [7, 1, 2, 2, 2],
            '8' => [7, 5, 7, 5, 7],
            '9' => [7, 5, 7, 1, 7],
            _ => [0, 2, 7, 2, 0],
        };
        for (row, bits) in rows.into_iter().enumerate() {
            for col in 0..3 {
                if bits & (4 >> col) == 0 {
                    continue;
                }
                for dy in 0..2 {
                    for dx in 0..wide {
                        let at =
                            ((3 + row * 2 + dy) * 16 + left + index * advance + col * wide + dx)
                                * 4;
                        pixels[at..at + 4].copy_from_slice(&[255; 4]);
                    }
                }
            }
        }
    }
    Some(pixels)
}

/// Renders the overlay at the window's physical DPI size, keeping glyph edges on pixel boundaries.
/// Sizes outside the native 16–128 pixel range are clamped to bound the allocation.
#[must_use]
pub fn badge_at_size(count: u32, size: u32) -> Option<Vec<u8>> {
    let base = badge(count)?;
    let size = size.clamp(BADGE_SIZE, 128) as usize;
    let mut pixels = vec![0; size * size * 4];
    for y in 0..size {
        for x in 0..size {
            let from = ((y * 16 / size) * 16 + x * 16 / size) * 4;
            let to = (y * size + x) * 4;
            pixels[to..to + 4].copy_from_slice(&base[from..from + 4]);
        }
    }
    Some(pixels)
}

/// Adds the tray dot; a template uses black so macOS supplies the menu bar's current tint.
#[must_use]
pub fn tray(base: &[u8], width: u32, height: u32, template: bool, waiting: bool) -> Vec<u8> {
    let mut pixels = base.to_vec();
    if !waiting || pixels.len() != (width * height * 4) as usize {
        return pixels;
    }
    let radius = (width.min(height) / 6).max(1);
    let color = if template {
        [0, 0, 0, 255]
    } else {
        [250, 80, 15, 255]
    };
    for y in height.saturating_sub(radius * 2)..height {
        for x in width.saturating_sub(radius * 2)..width {
            let dx = i64::from(x) - i64::from(width - radius);
            let dy = i64::from(y) - i64::from(height - radius);
            if dx * dx + dy * dy <= i64::from(radius * radius) {
                let at = ((y * width + x) * 4) as usize;
                pixels[at..at + 4].copy_from_slice(&color);
            }
        }
    }
    pixels
}
