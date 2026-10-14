//! Full-page PNG and the before/after comparison.
//!
//! Two implementations, chosen per platform:
//! - **Native, one render** (ShellX Canvas's idea, a whole-document render; ShellX Browser's Windows path):
//!   `WebKitGTK` `FullDocument` snapshot, `WebView2` `DevTools` `captureBeyondViewport` with a clip of the document size.
//! - **Stitch** (ShellX Browser's Linux/macOS path): scroll by one
//!   viewport, take the visible snapshot, paste; improved here: fixed and sticky elements are hidden before every
//!   slice after the first, also ones that appeared meanwhile (ShellX repeats a sticky header in every slice), slices keep the device pixels (ShellX resized to CSS
//!   pixels), and the page height is measured again after the lazy pass.
//!
//! Both run after a lazy pass: images marked `loading=lazy` load eagerly and the page is scrolled through once (lazy
//! loaders that watch the viewport), waiting a bounded time for pending images; the page's scroll position is restored.
//!
//! Depends on: vw-preview (`PreviewService` screenshots, `shot` decode/encode), `driver` (page scripts), tokio.
//! Called by: `capture`, tests.
//! Invariants: at most [`MAX_CSS_HEIGHT`] CSS pixels tall and [`MAX_PIXELS`] device pixels (cut, `truncated`);
//! every wait is bounded; the page's scroll position and the hidden elements are restored on every path.

use std::time::Duration;

use serde::Deserialize;
use vw_preview::{self as shot, Rgba};

use crate::driver::Browser;
use crate::scripts;

/// Tallest capture in CSS pixels.
pub const MAX_CSS_HEIGHT: u32 = 16_384;
/// Most device pixels in a capture (128 MiB of RGBA).
pub const MAX_PIXELS: u64 = 32_000_000;
/// Longest wait for lazy images.
const LAZY_WAIT: Duration = Duration::from_millis(2500);
/// Grid cell of the comparison's regions, in pixels.
const CELL: u32 = 32;
/// Settling after a scroll before a slice is taken.
const SLICE_SETTLE: Duration = Duration::from_millis(150);

/// How the full page was taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Method {
    /// One render of the whole document.
    Native,
    /// Viewport slices pasted together.
    Stitch,
    /// The comparison only: ShellX Browser's stitch as it is, without the lazy pass and
    /// without hiding fixed/sticky elements after the first slice.
    StitchPlain,
}

/// A full-page capture.
#[derive(Clone, Debug)]
pub struct FullShot {
    /// The PNG.
    pub png: Vec<u8>,
    /// Pixels.
    pub width: u32,
    /// Pixels.
    pub height: u32,
    /// The document's CSS size.
    pub css: (u32, u32),
    /// How.
    pub method: Method,
    /// Cut at the size limits.
    pub truncated: bool,
    /// Slices (stitch) or 1.
    pub slices: u32,
    /// Fixed/sticky elements hidden after the first slice (stitch).
    pub pinned: u32,
    /// Why the native capture was not used, when it was not.
    pub fallback: Option<String>,
}

/// The page facts `js/fullpage.js` answers (`metrics`; other ops fill parts).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct Metrics {
    pub(crate) url: String,
    pub(crate) title: String,
    width: f64,
    height: f64,
    viewport_width: f64,
    viewport_height: f64,
    client_width: f64,
    scroll_y: f64,
    pending_images: u64,
    pinned: u64,
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn px(v: f64) -> u32 {
    v.max(0.0).ceil().min(f64::from(u32::MAX)) as u32
}

/// One helper step (`metrics`, `scroll`, `eager`, `pin`, `unpin`), its answer bound to the authorized document.
pub(crate) async fn step(
    browser: &Browser,
    session_id: &str,
    op: &str,
    y: f64,
) -> Result<Metrics, String> {
    let page = browser.page_of(session_id).await?;
    let answer: Metrics = browser
        .run_script(session_id, scripts::fullpage(&page, op, y))
        .await?;
    browser.check_answer(session_id, &page, &answer.url)?;
    Ok(answer)
}

/// The lazy pass: eager images, one scroll through the page, a bounded wait for pending images, back to the top.
async fn lazy_pass(browser: &Browser, session_id: &str, first: &Metrics) -> Result<(), String> {
    step(browser, session_id, "eager", 0.0).await?;
    let view = first.viewport_height.max(200.0);
    let bottom = first.height.min(f64::from(MAX_CSS_HEIGHT));
    let mut y = 0.0;
    let mut steps = 0;
    while y < bottom && steps < 40 {
        y += view;
        steps += 1;
        step(browser, session_id, "scroll", y).await?;
        tokio::time::sleep(Duration::from_millis(60)).await;
    }
    step(browser, session_id, "scroll", 0.0).await?;
    let start = tokio::time::Instant::now();
    while start.elapsed() < LAZY_WAIT {
        if step(browser, session_id, "metrics", 0.0)
            .await?
            .pending_images
            == 0
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    Ok(())
}

/// Takes the full page of `session_id` (`prefer`: force one method, for the comparison; `None`: native when the
/// engine has it, else stitch).
///
/// # Errors
/// A plain reason (nothing open, the page did not answer, the engine failed).
pub async fn full_page(
    browser: &Browser,
    session_id: &str,
    prefer: Option<Method>,
) -> Result<FullShot, String> {
    let first = step(browser, session_id, "metrics", 0.0).await?;
    let original_y = first.scroll_y;
    let plain = prefer == Some(Method::StitchPlain);
    let result = async {
        if !plain {
            lazy_pass(browser, session_id, &first).await?;
        }
        let m = step(browser, session_id, "metrics", 0.0).await?;
        let ((css_w, css_h), cut) = clip_size(m.width, m.height);
        let mut fallback = None;
        if prefer != Some(Method::Stitch) && !plain {
            match browser
                .service()
                .screenshot_full_png(session_id, (css_w, css_h))
                .await
            {
                Ok(Some(png)) => {
                    let limit = usize::try_from(MAX_PIXELS * 4 * 2).unwrap_or(usize::MAX);
                    let image = shot::decode_limited(&png, limit)?;
                    return finish(image, (css_w, css_h), cut, Method::Native, 1, 0, None);
                }
                Ok(None) => fallback = Some("this platform has no one-render capture".to_owned()),
                Err(e) => fallback = Some(format!("the one-render capture failed: {e}")),
            }
            if prefer == Some(Method::Native) {
                return Err(fallback.unwrap_or_default());
            }
        }
        // The stitch is bounded by its own canvas (`stitch_canvas`): only the CSS height limit applies here, not the
        // one-render clip's (which also divides by a page-reported width).
        let full_h = px(m.height).max(1);
        let stitch_h = full_h.min(MAX_CSS_HEIGHT);
        stitch(
            browser,
            session_id,
            &m,
            stitch_h,
            full_h > stitch_h,
            fallback,
            plain,
        )
        .await
    }
    .await;
    let _ = step(browser, session_id, "unpin", 0.0).await;
    let _ = step(browser, session_id, "scroll", original_y).await;
    result
}

/// Cuts to the pixel cap and encodes the final PNG.
fn finish(
    mut image: Rgba,
    css: (u32, u32),
    cut: bool,
    method: Method,
    slices: u32,
    pinned: u32,
    fallback: Option<String>,
) -> Result<FullShot, String> {
    let mut truncated = cut;
    let max_h = u32::try_from(MAX_PIXELS / u64::from(image.width.max(1))).unwrap_or(u32::MAX);
    if image.height > max_h {
        image = crop(&image, image.width, max_h);
        truncated = true;
    }
    let png = shot::encode(&image)?;
    Ok(FullShot {
        png,
        width: image.width,
        height: image.height,
        css,
        method,
        truncated,
        slices,
        pinned,
        fallback,
    })
}

fn crop(image: &Rgba, width: u32, height: u32) -> Rgba {
    let (w, h) = (width.min(image.width), height.min(image.height));
    let mut pixels = Vec::with_capacity(w as usize * h as usize * 4);
    for y in 0..h {
        let start = (y as usize * image.width as usize) * 4;
        pixels.extend_from_slice(&image.pixels[start..start + w as usize * 4]);
    }
    Rgba {
        width: w,
        height: h,
        pixels,
    }
}

/// Widest one-render capture in CSS pixels.
pub const MAX_CSS_WIDTH: u32 = 4096;
/// Widest stitched canvas in device pixels.
pub const MAX_PIXEL_WIDTH: u32 = 8192;

/// The one-render capture's CSS size for a document the page reports as `width` x `height` (page data: any number,
/// NaN included), and whether it is cut. Capped on our side: at most [`MAX_CSS_WIDTH`] x
/// [`MAX_CSS_HEIGHT`] and a quarter of [`MAX_PIXELS`] CSS pixels.
#[must_use]
pub fn clip_size(width: f64, height: f64) -> ((u32, u32), bool) {
    let (full_w, full_h) = (px(width).max(1), px(height).max(1));
    let css_w = full_w.min(MAX_CSS_WIDTH);
    // The engine renders the clip at its device pixel ratio (a page cannot change it, but it is not known here):
    // a quarter of the pixel cap in CSS pixels keeps a ratio-2 screen within the cap; above that the decode limit
    // refuses the image and the capture falls back to the stitch.
    let room = u32::try_from(MAX_PIXELS / 4 / u64::from(css_w)).unwrap_or(u32::MAX);
    let css_h = full_h.min(MAX_CSS_HEIGHT).min(room).max(1);
    ((css_w, css_h), full_w > css_w || full_h > css_h)
}

/// The stitched canvas for slices of `slice_w` x `slice_h` device pixels (our own snapshot) of a page reporting
/// `viewport_w` and `client_w` (CSS; page data, may be redefined by the page's script), covering `css_h` CSS pixels:
/// (width, height, device pixels per CSS pixel). Capped on our side before anything is allocated: the scale stays within 0.25..=4, the width within the slice and [`MAX_PIXEL_WIDTH`], the area within
/// [`MAX_PIXELS`].
#[must_use]
pub fn stitch_canvas(
    slice_w: u32,
    slice_h: u32,
    viewport_w: f64,
    client_w: f64,
    css_h: u32,
) -> (u32, u32, f64) {
    let raw = f64::from(slice_w) / viewport_w.max(1.0);
    let scale = if raw.is_finite() {
        raw.clamp(0.25, 4.0)
    } else {
        1.0
    };
    let width = px(client_w.max(1.0) * scale).clamp(1, slice_w.clamp(1, MAX_PIXEL_WIDTH));
    let room = u32::try_from(MAX_PIXELS / u64::from(width)).unwrap_or(u32::MAX);
    let height = px(f64::from(css_h) * scale)
        .min(room)
        .max(slice_h.min(room))
        .max(1);
    (width, height, scale)
}

/// Scroll-and-stitch. `m`: metrics after the lazy pass; `css_h`: the height to cover.
async fn stitch(
    browser: &Browser,
    session_id: &str,
    m: &Metrics,
    css_h: u32,
    cut: bool,
    fallback: Option<String>,
    plain: bool,
) -> Result<FullShot, String> {
    let view_h = m.viewport_height.max(1.0);
    let mut canvas: Option<Rgba> = None;
    let mut scale = 1.0;
    let mut y = 0.0;
    let mut slices = 0u32;
    let mut pinned = 0u32;
    let max_y = (f64::from(css_h) - view_h).max(0.0);
    loop {
        let at = step(browser, session_id, "scroll", y).await?;
        tokio::time::sleep(SLICE_SETTLE).await;
        // Fixed and sticky elements show in the first slice only: before every later slice, whatever is fixed or
        // sticky NOW is hidden (a cookie banner can appear mid-capture and repeat in later slices
        // when hidden only once after the first slice).
        if slices >= 1 && !plain {
            let now = step(browser, session_id, "pin", 0.0).await?.pinned;
            pinned = pinned.saturating_add(u32::try_from(now).unwrap_or(0));
        }
        let png = browser
            .service()
            .screenshot_png(session_id)
            .await
            .map_err(|e| e.to_string())?;
        let slice = shot::decode(&png)?;
        if canvas.is_none() {
            // Device pixels per CSS pixel: the snapshot covers the viewport (scrollbar included on some engines).
            let (width, height, s) = stitch_canvas(
                slice.width,
                slice.height,
                m.viewport_width,
                m.client_width,
                css_h,
            );
            scale = s;
            canvas = Some(Rgba {
                width,
                height,
                pixels: vec![0; width as usize * height as usize * 4],
            });
        }
        let Some(target) = canvas.as_mut() else {
            break;
        };
        let top = px(at.scroll_y * scale);
        paste(target, &slice, top);
        slices += 1;
        if y >= max_y || slices >= 200 {
            break;
        }
        // One viewport further, measured from our own snapshot (the page's innerHeight is page data).
        let step = (f64::from(slice.height) / scale).max(50.0);
        y = (y + step).min(max_y);
    }
    let image = canvas.ok_or("no slice was taken")?;
    finish(
        image,
        (px(m.width).min(MAX_CSS_WIDTH), css_h),
        cut,
        if plain {
            Method::StitchPlain
        } else {
            Method::Stitch
        },
        slices,
        pinned,
        fallback,
    )
}

/// Pastes `slice` into `target` at row `top` (rows past the target's bottom are dropped).
fn paste(target: &mut Rgba, slice: &Rgba, top: u32) {
    let w = target.width.min(slice.width) as usize * 4;
    for row in 0..slice.height {
        let y = top + row;
        if y >= target.height {
            break;
        }
        let dst = y as usize * target.width as usize * 4;
        let src = row as usize * slice.width as usize * 4;
        target.pixels[dst..dst + w].copy_from_slice(&slice.pixels[src..src + w]);
    }
}

/// The result of comparing two captures.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Comparison {
    /// Share of compared pixels that changed (0..=1).
    pub changed_share: f64,
    /// Changed regions in the after image's pixels (`x, y, width, height`), largest first, at most 10.
    pub regions: Vec<[u32; 4]>,
    /// Before and after sizes.
    pub before_size: [u32; 2],
    /// After size.
    pub after_size: [u32; 2],
    /// The diff picture: the after image dimmed, changed pixels in red.
    #[serde(skip)]
    pub diff_png: Vec<u8>,
}

/// Compares two PNGs pixel by pixel over their common area (a channel differing by more than 24 counts), groups the
/// changes into regions on a 32-pixel grid, and draws the diff picture.
///
/// # Errors
/// A plain reason when a picture cannot be decoded.
pub fn compare(before: &[u8], after: &[u8]) -> Result<Comparison, String> {
    let limit = usize::try_from(MAX_PIXELS * 4 * 2).unwrap_or(usize::MAX);
    let (a, b) = (
        shot::decode_limited(before, limit)?,
        shot::decode_limited(after, limit)?,
    );
    let (w, h) = (a.width.min(b.width), a.height.min(b.height));
    let (gw, gh) = (b.width.div_ceil(CELL), b.height.div_ceil(CELL));
    let mut grid = vec![false; gw as usize * gh as usize];
    let mut diff = b.pixels.clone();
    for p in diff.as_chunks_mut::<4>().0 {
        for c in &mut p[..3] {
            // Dimmed towards light grey: 150 + 40% of the value, at most 252.
            *c = u8::try_from(u16::from(*c) * 2 / 5 + 150).unwrap_or(u8::MAX);
        }
    }
    let mut changed = 0u64;
    for y in 0..b.height {
        for x in 0..b.width {
            let at = (y as usize * b.width as usize + x as usize) * 4;
            let differs = if x < w && y < h {
                let ap = (y as usize * a.width as usize + x as usize) * 4;
                (0..3).any(|c| a.pixels[ap + c].abs_diff(b.pixels[at + c]) > 24)
            } else {
                true
            };
            if differs {
                if x < w && y < h {
                    changed += 1;
                }
                grid[(y / CELL) as usize * gw as usize + (x / CELL) as usize] = true;
                diff[at..at + 4].copy_from_slice(&[230, 30, 30, 255]);
            }
        }
    }
    let regions = regions(&grid, gw, gh, CELL, (b.width, b.height));
    let total = u64::from(w) * u64::from(h);
    #[allow(clippy::cast_precision_loss)]
    let share = if total == 0 {
        0.0
    } else {
        changed as f64 / total as f64
    };
    let diff_png = shot::encode(&Rgba {
        width: b.width,
        height: b.height,
        pixels: diff,
    })?;
    Ok(Comparison {
        changed_share: (share * 10_000.0).round() / 10_000.0,
        regions,
        before_size: [a.width, a.height],
        after_size: [b.width, b.height],
        diff_png,
    })
}

/// Connected groups of changed grid cells as pixel rectangles, largest first, at most 10.
fn regions(grid: &[bool], gw: u32, gh: u32, cell: u32, size: (u32, u32)) -> Vec<[u32; 4]> {
    let mut seen = vec![false; grid.len()];
    let mut out: Vec<[u32; 4]> = Vec::new();
    for start in 0..grid.len() {
        if !grid[start] || seen[start] {
            continue;
        }
        let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
        let mut stack = vec![start];
        seen[start] = true;
        while let Some(i) = stack.pop() {
            let cell_index = u32::try_from(i).unwrap_or(u32::MAX);
            let (x, y) = (cell_index % gw, cell_index / gw);
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
            let mut near = Vec::with_capacity(4);
            if x > 0 {
                near.push(i - 1);
            }
            if x + 1 < gw {
                near.push(i + 1);
            }
            if y > 0 {
                near.push(i - gw as usize);
            }
            if y + 1 < gh {
                near.push(i + gw as usize);
            }
            for n in near {
                if grid[n] && !seen[n] {
                    seen[n] = true;
                    stack.push(n);
                }
            }
        }
        let (px0, py0) = (x0 * cell, y0 * cell);
        let (px1, py1) = (((x1 + 1) * cell).min(size.0), ((y1 + 1) * cell).min(size.1));
        out.push([px0, py0, px1 - px0, py1 - py0]);
    }
    out.sort_by_key(|r| std::cmp::Reverse(u64::from(r[2]) * u64::from(r[3])));
    out.truncate(10);
    out
}
