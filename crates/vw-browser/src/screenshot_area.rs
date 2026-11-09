//! Viewport, element and rectangle screenshots using the shared native capture and document binding.
//! Called by `browser_act`; crops CSS coordinates only after checking the page and element again.
use crate::{
    ActError, Actor, Browser,
    driver::{failed, refused},
};
use serde_json::{Value, json};
use vw_preview::{PixelRect, crop, decode, encode};

/// The portion of the current page to capture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Area {
    /// The visible viewport.
    Viewport,
    /// One observed element in the viewport.
    Element(String),
    /// A rectangle in top-level CSS pixels.
    Rectangle(crate::annotate::ViewRect),
}

impl Area {
    pub(crate) fn parse(args: &Value) -> Result<Self, String> {
        if args.get("ref").is_some() && args.get("clip").is_some() {
            return Err("Capture takes either ref or clip, not both.".into());
        }
        if let Some(reference) = args.get("ref") {
            return reference
                .as_str()
                .filter(|r| crate::model::is_ref(r))
                .map(|r| Self::Element(r.into()))
                .ok_or_else(|| "Capture needs a current element ref from browser_read.".into());
        }
        if let Some(clip) = args.get("clip") {
            let rect: crate::annotate::ViewRect = serde_json::from_value(clip.clone())
                .map_err(|_| "clip needs integer x, y, width and height in viewport CSS pixels.")?;
            if rect.x < 0
                || rect.y < 0
                || rect.width <= 0
                || rect.height <= 0
                || [rect.x, rect.y, rect.width, rect.height]
                    .iter()
                    .any(|v| *v > 32768)
            {
                return Err("clip needs nonnegative x/y and positive width/height, each at most 32768 CSS pixels.".into());
            }
            return Ok(Self::Rectangle(rect));
        }
        Ok(Self::Viewport)
    }

    pub(crate) fn reference(&self) -> Option<&str> {
        match self {
            Self::Element(r) => Some(r),
            _ => None,
        }
    }
}

impl Browser {
    pub(crate) async fn screenshot_area(
        &self,
        session: &str,
        area: &Area,
        by: Actor,
    ) -> Result<Value, ActError> {
        let bound = self.bind_read(session, by).await.map_err(refused)?;
        let before = bound.observe_dom(session).await.map_err(refused)?;
        let viewport = before.viewport.ok_or_else(|| {
            refused("The page has no readable viewport; read it again before capturing.")
        })?;
        let element = if let Some(reference) = area.reference() {
            let seen = self
                .current_seen(session)
                .and_then(|s| s.items.into_iter().find(|r| r.id == reference))
                .ok_or_else(|| refused("The capture ref is stale; read the page again."))?;
            Some(
                before
                    .refs
                    .iter()
                    .find(|r| r.id == reference && crate::point::same_node(&seen, r))
                    .ok_or_else(|| refused("The capture element changed; read the page again."))?,
            )
        } else {
            None
        };
        let rect = match area {
            Area::Viewport => PixelRect {
                x: 0,
                y: 0,
                width: viewport.0,
                height: viewport.1,
            },
            Area::Rectangle(r) => PixelRect {
                x: i64::from(r.x),
                y: i64::from(r.y),
                width: u32::try_from(r.width).unwrap_or(0),
                height: u32::try_from(r.height).unwrap_or(0),
            },
            Area::Element(_) => {
                let item = element.ok_or_else(|| {
                    refused("The capture element disappeared; read the page again.")
                })?;
                let bounds=item.bounds.filter(|_|item.visible!=Some(false)).ok_or_else(||refused("The element has no visible rectangle; scroll it into view before capturing."))?;
                PixelRect {
                    x: i64::from(bounds.x),
                    y: i64::from(bounds.y),
                    width: u32::try_from(bounds.width).unwrap_or(0),
                    height: u32::try_from(bounds.height).unwrap_or(0),
                }
            }
        };
        if rect.width == 0
            || rect.height == 0
            || rect.x < 0
            || rect.y < 0
            || rect.x + i64::from(rect.width) > i64::from(viewport.0)
            || rect.y + i64::from(rect.height) > i64::from(viewport.1)
        {
            return Err(refused(
                "The requested area is outside the visible viewport; scroll it into view or choose a smaller rectangle before capturing.",
            ));
        }
        let png = self
            .service
            .screenshot_png(session)
            .await
            .map_err(|e| failed(crate::driver::preview_text(&e)))?;
        let after = bound.observe_dom(session).await.map_err(refused)?;
        bound.check_read_mount().map_err(refused)?;
        if before.document != after.document
            || before.url != after.url
            || before.viewport != after.viewport
            || element.is_some_and(|old| {
                !after
                    .refs
                    .iter()
                    .any(|new| crate::point::same_node(old, new) && old.bounds == new.bounds)
            })
        {
            return Err(refused(
                "The page or capture element changed during the screenshot; read it again before capturing.",
            ));
        }
        let image = decode(&png).map_err(failed)?;
        let scale_x =
            |x: i64| x.saturating_mul(i64::from(image.width)) / i64::from(viewport.0.max(1));
        let scale_y =
            |y: i64| y.saturating_mul(i64::from(image.height)) / i64::from(viewport.1.max(1));
        let pixels = PixelRect {
            x: scale_x(rect.x),
            y: scale_y(rect.y),
            width: u32::try_from(scale_x(i64::from(rect.width))).unwrap_or(0),
            height: u32::try_from(scale_y(i64::from(rect.height))).unwrap_or(0),
        };
        let cropped = crop(&image, pixels).ok_or_else(|| {
            failed("The requested screenshot area has no pixels; choose a larger rectangle.")
        })?;
        let shot = vw_preview::shape(&encode(&cropped).map_err(failed)?).map_err(failed)?;
        Ok(receipt(&shot, rect))
    }
}

fn receipt(shot: &vw_preview::Shot, rect: PixelRect) -> Value {
    // Report the final image's mapping, including integer crop rounding and the existing image-size cap,
    // rather than trusting page devicePixelRatio. Width/height retain their device-pixel meaning.
    json!({
        "Outcome":"Screenshot captured.", "width":shot.width, "height":shot.height,
        "cssRect":{"x":rect.x,"y":rect.y,"width":rect.width,"height":rect.height},
        "pixelSize":{"width":shot.width,"height":shot.height},
        "scale":{"x":f64::from(shot.width)/f64::from(rect.width),"y":f64::from(shot.height)/f64::from(rect.height)},
        "vwImage":{"mimeType":"image/png","data":shot.base64()}
    })
}
