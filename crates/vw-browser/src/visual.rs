//! Bounded deterministic screenshot regions for canvas and image gaps in the DOM pass.
//! Uses preview captures and pixel components only (no text recognition: the agent's model reads the screenshot);
//! called only for visible uncovered regions.
use crate::{
    Browser,
    frames::RefBox,
    model::{PageModel, RefItem},
};
use std::time::{Duration, Instant};
use vw_preview::{PixelRect, Rgba};

impl Browser {
    pub(crate) async fn visual_refs(&self, session: &str, model: &mut PageModel) {
        let regions: Vec<_> = model
            .refs
            .iter()
            .filter(|r| {
                r.coordinate_kind.is_some()
                    && r.offscreen != Some(true)
                    && r.visible != Some(false)
                    && r.frame_id.is_some()
            })
            .take(8)
            .cloned()
            .collect();
        if regions.is_empty() {
            return;
        }
        let Some((width, height)) = model.viewport.filter(|(w, h)| *w > 0 && *h > 0) else {
            return;
        };
        let Ok(Ok(shot)) = tokio::time::timeout(
            Duration::from_millis(1200),
            self.service.screenshot(session),
        )
        .await
        else {
            return;
        };
        let Ok(image) = vw_preview::decode(&shot.png) else {
            return;
        };
        let started = Instant::now();
        for region in regions {
            if started.elapsed() > Duration::from_millis(600) || model.refs.len() >= 200 {
                break;
            }
            let Some(bounds) = region.bounds else {
                continue;
            };
            let pixels = pixel_box(bounds, (width, height), (image.width, image.height));
            let Some(crop) = vw_preview::crop(&image, pixels) else {
                continue;
            };
            let edge = crop.width.max(crop.height).max(1024);
            let sample = (crop.width * 1024 / edge, crop.height * 1024 / edge);
            let crop = vw_preview::fit(&crop, sample.0.max(1), sample.1.max(1));
            let sample = (crop.width, crop.height);
            for (kind, rect) in shapes(&crop, started) {
                let rect = css_box(
                    rect,
                    pixels,
                    sample,
                    (width, height),
                    (image.width, image.height),
                );
                model
                    .refs
                    .push(derived(&region, kind, &format!("{kind} region"), rect));
            }
        }
        model.all_refs = model.refs.iter().map(|r| r.id.clone()).collect();
        model.ref_count = u32::try_from(model.refs.len()).unwrap_or(200);
    }
}

fn derived(parent: &RefItem, kind: &str, label: &str, bounds: RefBox) -> RefItem {
    let mut item = parent.clone();
    item.id = format!(
        "e{}",
        &crate::observe::hash(&[&parent.id, kind, &format!("{bounds:?}")])[..8]
    );
    item.coordinate_kind = Some(kind.into());
    item.name = label.chars().take(160).collect();
    item.role = kind.into();
    item.bounds = Some(bounds);
    item
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn pixel_box(b: RefBox, css: (u32, u32), image: (u32, u32)) -> PixelRect {
    let rect = PixelRect {
        x: (f64::from(b.x) * f64::from(image.0) / f64::from(css.0)).round() as i64,
        y: (f64::from(b.y) * f64::from(image.1) / f64::from(css.1)).round() as i64,
        width: (f64::from(b.width.max(0)) * f64::from(image.0) / f64::from(css.0)).round() as u32,
        height: (f64::from(b.height.max(0)) * f64::from(image.1) / f64::from(css.1)).round() as u32,
    };
    let x = rect.x.clamp(0, i64::from(image.0));
    let y = rect.y.clamp(0, i64::from(image.1));
    PixelRect {
        x,
        y,
        width: (rect.x + i64::from(rect.width))
            .clamp(x, i64::from(image.0))
            .saturating_sub(x) as u32,
        height: (rect.y + i64::from(rect.height))
            .clamp(y, i64::from(image.1))
            .saturating_sub(y) as u32,
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
fn css_box(
    mut b: PixelRect,
    crop: PixelRect,
    sample: (u32, u32),
    css: (u32, u32),
    image: (u32, u32),
) -> RefBox {
    b.x = b.x * i64::from(crop.width) / i64::from(sample.0);
    b.y = b.y * i64::from(crop.height) / i64::from(sample.1);
    b.width = b.width * crop.width / sample.0;
    b.height = b.height * crop.height / sample.1;
    RefBox {
        x: ((b.x + crop.x) as f64 * f64::from(css.0) / f64::from(image.0)).round() as i32,
        y: ((b.y + crop.y) as f64 * f64::from(css.1) / f64::from(image.1)).round() as i32,
        width: (f64::from(b.width) * f64::from(css.0) / f64::from(image.0)).round() as i32,
        height: (f64::from(b.height) * f64::from(css.1) / f64::from(image.1)).round() as i32,
    }
}

fn shapes(image: &Rgba, started: Instant) -> Vec<(&'static str, PixelRect)> {
    let (w, h) = (image.width as usize, image.height as usize);
    if w == 0 || h == 0 || w * h > 1024 * 1024 {
        return vec![];
    }
    let background = &image.pixels[..3];
    let foreground = |at: usize| {
        image.pixels[at * 4..at * 4 + 3]
            .iter()
            .zip(background)
            .any(|(a, b)| a.abs_diff(*b) > 80)
    };
    let mut seen = vec![false; w * h];
    let mut out = Vec::new();
    for at in 0..w * h {
        if at % 1024 == 0 && started.elapsed() > Duration::from_millis(300) {
            break;
        }
        if seen[at] || !foreground(at) {
            continue;
        }
        let mut pending = vec![at];
        seen[at] = true;
        let (mut left, mut top, mut right, mut bottom) = (at % w, at / w, at % w, at / w);
        let mut visited = 0usize;
        while let Some(p) = pending.pop() {
            visited += 1;
            if visited.is_multiple_of(1024) && started.elapsed() > Duration::from_millis(300) {
                return out;
            }
            let (x, y) = (p % w, p / w);
            left = left.min(x);
            right = right.max(x);
            top = top.min(y);
            bottom = bottom.max(y);
            for q in [
                p.checked_sub(w),
                (y + 1 < h).then_some(p + w),
                (x > 0).then(|| p - 1),
                (x + 1 < w).then_some(p + 1),
            ]
            .into_iter()
            .flatten()
            {
                if !seen[q] && foreground(q) {
                    seen[q] = true;
                    pending.push(q);
                }
            }
        }
        let (width, height) = (right - left + 1, bottom - top + 1);
        if !(10..=512).contains(&width) || !(10..=160).contains(&height) {
            continue;
        }
        let border = (left..=right)
            .filter(|x| foreground(top * w + x) && foreground(bottom * w + x))
            .count()
            + (top..=bottom)
                .filter(|y| foreground(y * w + left) && foreground(y * w + right))
                .count();
        if border * 10 < (width + height) * 8 {
            continue;
        }
        let kind = if width.abs_diff(height) * 4 <= height {
            "checkbox"
        } else if width > height && width < height * 10 {
            "button"
        } else {
            continue;
        };
        out.push((
            kind,
            PixelRect {
                x: i64::try_from(left).unwrap_or(0),
                y: i64::try_from(top).unwrap_or(0),
                width: u32::try_from(width).unwrap_or(0),
                height: u32::try_from(height).unwrap_or(0),
            },
        ));
        if out.len() == 32 {
            break;
        }
    }
    out
}
