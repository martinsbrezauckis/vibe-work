//! Bounded native viewport snapshots for full-page stitching on `WebKitGTK`.
//! Depends on GTK allocation and page zoom; called by platform capture, preserving the CSS viewport.

use std::time::Duration;

use gtk::prelude::*;
use javascriptcore::ValueExt as _;
use tauri::{Webview, Wry};
use tokio::sync::OwnedMutexGuard;
use vw_preview::EngineError;
use webkit2gtk::{SnapshotOptions, SnapshotRegion, WebViewExt as _};

const WAIT: Duration = Duration::from_secs(2);
const WORLD: &str = "vibe-work-capture";
const METRICS: &str = "[innerWidth, innerHeight, scrollX, scrollY]";

fn capture_size(viewport: (i32, i32), zoom: f64, dpr: f64) -> Result<(i32, i32), String> {
    if viewport.0 <= 0
        || viewport.1 <= 0
        || !dpr.is_finite()
        || dpr <= 0.0
        || !zoom.is_finite()
        || zoom <= 0.0
    {
        return Err("invalid capture viewport".to_owned());
    }
    let width = (f64::from(viewport.0) / zoom).ceil();
    let height = (f64::from(viewport.1) / zoom).ceil();
    if width * dpr > 8192.0
        || width * height * dpr * dpr > 32_000_000.0
        || width > f64::from(i32::MAX)
        || height > f64::from(i32::MAX)
    {
        return Err("capture viewport exceeds the pixel limits".to_owned());
    }
    // Checked before conversion/allocation, including the screen scale (which excludes page zoom).
    #[allow(clippy::cast_possible_truncation)]
    Ok((width as i32, height as i32))
}

async fn visible(view: &webkit2gtk::WebView) -> Result<cairo::Surface, String> {
    glib::future_with_timeout(
        WAIT,
        view.snapshot_future(SnapshotRegion::Visible, SnapshotOptions::NONE),
    )
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())
}

async fn script(view: &webkit2gtk::WebView, code: &str) -> Result<[f64; 4], String> {
    let value = glib::future_with_timeout(
        WAIT,
        view.evaluate_javascript_future(code, Some(WORLD), None),
    )
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;
    let json = value
        .to_json(0)
        .ok_or("the capture viewport did not answer")?;
    let metrics: [f64; 4] = serde_json::from_str(&json).map_err(|e| e.to_string())?;
    if metrics.iter().any(|v| !v.is_finite()) {
        return Err("invalid capture viewport metrics".to_owned());
    }
    Ok(metrics)
}

async fn scroll(view: &webkit2gtk::WebView, original: [f64; 4]) -> Result<(), String> {
    let code = format!(
        "scrollTo({{left: {}, top: {}, behavior: 'instant'}}); {METRICS}",
        original[2], original[3]
    );
    let current = script(view, &code).await?;
    if current
        .iter()
        .zip(original)
        .any(|(a, b)| (a - b).abs() > 1.0)
    {
        return Err("the capture viewport or scroll changed".to_owned());
    }
    Ok(())
}

struct Parked {
    view: webkit2gtk::WebView,
    layer: gtk::Fixed,
    cover: gtk::Image,
    geometry: crate::linux_layout::Geometry,
    request: (i32, i32),
    zoom: f64,
    visible: bool,
    child_visible: bool,
}

impl Parked {
    fn new(view: &webkit2gtk::WebView, surface: &cairo::Surface) -> Result<Self, String> {
        let layer = view
            .parent()
            .and_then(|p| p.downcast::<gtk::Fixed>().ok())
            .filter(|p| p.widget_name() == "vw-preview-layer")
            .ok_or("the preview has no capture layer")?;
        let allocation = view.allocation();
        let cover = gtk::Image::from_surface(Some(surface));
        let geometry = (
            layer.child_x(view),
            layer.child_y(view),
            allocation.width(),
            allocation.height(),
        );
        cover.set_size_request(geometry.2, geometry.3);
        layer.put(&cover, geometry.0, geometry.1);
        if view.is_mapped() {
            cover.show();
        }
        Ok(Self {
            view: view.clone(),
            layer,
            cover,
            geometry,
            request: view.size_request(),
            zoom: view.zoom_level(),
            visible: view.is_visible(),
            child_visible: view.is_child_visible(),
        })
    }

    fn restore_geometry(&self) {
        self.view.set_zoom_level(self.zoom);
        let _ = crate::linux_layout::show_at(
            &self.view,
            (-30_000, -30_000, self.geometry.2, self.geometry.3),
        );
    }
}

impl Drop for Parked {
    fn drop(&mut self) {
        self.restore_geometry();
        let _ = crate::linux_layout::place(&self.view, self.geometry);
        self.view.set_size_request(self.request.0, self.request.1);
        self.view.set_child_visible(self.child_visible);
        if !self.visible {
            self.view.hide();
        }
        self.layer.remove(&self.cover);
    }
}

async fn scaled(view: &webkit2gtk::WebView) -> Result<cairo::Surface, String> {
    let zoom = view.zoom_level();
    if (zoom - 1.0).abs() < f64::EPSILON {
        return visible(view).await;
    }
    let a = view.allocation();
    let size = capture_size(
        (a.width(), a.height()),
        zoom,
        f64::from(view.scale_factor()),
    )?;
    let surface = visible(view).await?;
    let metrics = script(view, METRICS).await?;
    let parked = Parked::new(view, &surface)?;
    let result = glib::future_with_timeout(WAIT, async {
        crate::linux_layout::show_at(view, (-30_000, -30_000, size.0, size.1))?;
        view.set_zoom_level(1.0);
        glib::timeout_future(Duration::from_millis(60)).await;
        // Allocation and zoom reach WebKit asynchronously. Wait for the new viewport before scrolling, or the
        // intermediate, taller viewport clamps a bottom slice's scroll position and drops the page's last rows.
        visible(view).await?;
        scroll(view, metrics).await?;
        visible(view).await
    })
    .await
    .map_err(|e| e.to_string())
    .and_then(std::convert::identity);
    parked.restore_geometry();
    // Keep the old frame over the parked widget until WebKit has rendered at its original zoom again.
    let restored = glib::future_with_timeout(WAIT, async {
        glib::timeout_future(Duration::from_millis(60)).await;
        visible(view).await?;
        scroll(view, metrics).await?;
        visible(view).await?;
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())
    .and_then(std::convert::identity);
    drop(parked);
    result.and_then(|image| restored.map(|()| image))
}

pub(crate) async fn screenshot(
    webview: &Webview<Wry>,
    limit: Duration,
    visibility: OwnedMutexGuard<()>,
) -> Result<Vec<u8>, EngineError> {
    crate::platform::with_native(
        webview,
        "taking a full-resolution slice",
        limit,
        move |native, reply| {
            glib::MainContext::default().spawn_local(async move {
                // This task outlives its awaiting caller: every temporary native change is restored before unlock.
                let _visibility = visibility;
                if reply.is_closed() {
                    return;
                }
                let view = native.inner();
                let surface = scaled(&view).await;
                let result = surface.and_then(|surface| {
                    let mut bytes = Vec::new();
                    surface
                        .write_to_png(&mut bytes)
                        .map_err(|e| e.to_string())?;
                    Ok(bytes)
                });
                let _ = reply.send(result);
            });
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::capture_size;

    #[test]
    fn css_pixels_times_dpr_at_every_zoom() {
        for zoom in [0.5, 0.6, 1.0, 2.0] {
            for dpr in [1.0, 1.5, 2.0] {
                #[allow(clippy::cast_possible_truncation)]
                let viewport = ((1280.0 * zoom) as i32, (800.0 * zoom) as i32);
                let logical = capture_size(viewport, zoom, dpr).unwrap();
                assert_eq!(logical, (1280, 800), "zoom {zoom}, DPR {dpr}");
                assert_eq!(f64::from(logical.0) * dpr, 1280.0 * dpr);
                assert_eq!(f64::from(logical.1) * dpr, 800.0 * dpr);
            }
        }
    }

    #[test]
    fn pixel_limits_are_checked_before_enlarging_the_viewport() {
        assert_eq!(capture_size((4096, 1953), 1.0, 2.0).unwrap(), (4096, 1953));
        assert!(capture_size((4097, 10), 1.0, 2.0).is_err());
        assert!(capture_size((4096, 1954), 1.0, 2.0).is_err());
        assert!(capture_size((4096, 100), 0.5, 2.0).is_err());
        assert!(capture_size((640, 12_000), 0.5, 2.0).is_err());
    }

    #[test]
    fn invalid_geometry_is_refused() {
        for zoom in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(capture_size((640, 400), zoom, 1.0).is_err());
        }
        for dpr in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(capture_size((640, 400), 1.0, dpr).is_err());
        }
        assert!(capture_size((0, 400), 1.0, 1.0).is_err());
    }
}
