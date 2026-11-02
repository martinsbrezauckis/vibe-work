//! Ref labels rasterized on the agent's screenshot copy, after capture. Never changes page DOM or styles.
//! Uses a small original digit font and preview PNG helpers; driver supplies only refs present in its answer.
use crate::model::PageModel;
use vw_preview::{Rgba, Shot};

const DIGITS: [[u8; 5]; 10] = [
    [7, 5, 5, 5, 7],
    [2, 6, 2, 2, 7],
    [7, 1, 7, 4, 7],
    [7, 1, 7, 1, 7],
    [5, 5, 7, 1, 1],
    [7, 4, 7, 1, 7],
    [7, 4, 7, 5, 7],
    [7, 1, 1, 1, 1],
    [7, 5, 7, 5, 7],
    [7, 5, 7, 1, 7],
];

pub(crate) fn annotate(
    shot: Shot,
    model: &PageModel,
    answer: &serde_json::Value,
) -> Result<Shot, String> {
    let Some((width, height)) = model.viewport.filter(|(w, h)| *w > 0 && *h > 0) else {
        return Ok(shot);
    };
    let Some(refs) = answer["untrustedPageContent"]["refs"].as_array() else {
        return Ok(shot);
    };
    let mut image = vw_preview::decode(&shot.png)?;
    for item in &model.refs {
        if item.visible == Some(false)
            || item.offscreen == Some(true)
            || !refs.iter().any(|r| r["ref"] == item.id)
        {
            continue;
        }
        let (Some(rect), Some(label)) = (item.bounds, item.label.as_deref()) else {
            continue;
        };
        if rect.width <= 0
            || rect.height <= 0
            || rect.x < 0
            || rect.y < 0
            || i64::from(rect.x) >= i64::from(width)
            || i64::from(rect.y) >= i64::from(height)
        {
            continue;
        }
        let x = i64::from(rect.x) * i64::from(image.width) / i64::from(width);
        let y = i64::from(rect.y) * i64::from(image.height) / i64::from(height);
        draw(&mut image, label, x, (y - 15).max(0));
    }
    Ok(Shot {
        width: image.width,
        height: image.height,
        png: vw_preview::encode(&image)?,
    })
}

fn pixel(image: &mut Rgba, x: i64, y: i64, color: [u8; 4]) {
    if x < 0 || y < 0 || x >= i64::from(image.width) || y >= i64::from(image.height) {
        return;
    }
    let at = usize::try_from((y * i64::from(image.width) + x) * 4).unwrap_or(0);
    image.pixels[at..at + 4].copy_from_slice(&color);
}

fn draw(image: &mut Rgba, text: &str, x: i64, y: i64) {
    let width = i64::try_from(text.len()).unwrap_or(3) * 8 + 4;
    for row in 0..14 {
        for col in 0..width {
            pixel(image, x + col, y + row, [20, 35, 70, 255]);
        }
    }
    for (index, digit) in text.bytes().enumerate() {
        let Some(glyph) = digit
            .checked_sub(b'0')
            .and_then(|n| DIGITS.get(usize::from(n)))
        else {
            continue;
        };
        for (row, bits) in glyph.iter().enumerate() {
            for col in 0..3 {
                if bits & (1 << (2 - col)) == 0 {
                    continue;
                }
                for dy in 0..2 {
                    for dx in 0..2 {
                        pixel(
                            image,
                            x + 2 + i64::try_from(index).unwrap_or(0) * 8 + col * 2 + dx,
                            y + 2 + i64::try_from(row).unwrap_or(0) * 2 + dy,
                            [255, 255, 255, 255],
                        );
                    }
                }
            }
        }
    }
}
