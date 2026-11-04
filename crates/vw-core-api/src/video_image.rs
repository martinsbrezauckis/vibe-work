//! Saved sheet cells projected into the existing image viewer. No native decoding in core;
//! a bounded SVG viewport contains only the verified PNG and integer geometry, never media text.
use crate::{files_read::FileKind, files_view::FileContent, video_history::Record};
use base64::Engine as _;
use serde_json::{Value, json};

pub(crate) fn projection(meta: &Value) -> Value {
    let mut view = meta.clone();
    if let Some(map) = view.as_object_mut() {
        map.remove(vw_agent_tools::wire::IMAGE_KEY);
    }
    view["plannedFrames"] = meta["planned_frames"].clone();
    view["imageCount"] = meta["image_count"].clone();
    if meta["image_count"] != 1 {
        view["frames"] = json!([]);
        return view;
    }
    if let Some(frames) = view["frames"].as_array_mut() {
        for frame in frames {
            frame["file"] = json!(format!(
                "video:{}:{}",
                meta["video_id"].as_str().unwrap_or_default(),
                frame["frame_id"]
            ));
        }
    }
    view
}
pub(super) fn content(record: &Record, handle: &str, cell: usize) -> Result<FileContent, String> {
    let meta = &record.metadata;
    let frames = meta["frames"]
        .as_array()
        .ok_or("Invalid saved video metadata.")?;
    let frame = frames
        .iter()
        .find(|f| f["frame_id"].as_u64() == Some(cell as u64))
        .ok_or("Unknown saved video frame.")?;
    let w = meta["image_size"][0]
        .as_u64()
        .filter(|v| *v > 0 && *v <= 1280)
        .ok_or("Invalid saved image size.")?;
    let h = meta["image_size"][1]
        .as_u64()
        .filter(|v| *v > 0 && *v <= 1280)
        .ok_or("Invalid saved image size.")?;
    let cols = meta["columns"]
        .as_u64()
        .filter(|v| *v > 0 && *v <= 4)
        .ok_or("Invalid saved sheet.")?;
    let count = meta
        .get("selected_frames")
        .unwrap_or(&meta["planned_frames"])
        .as_u64()
        .filter(|v| *v > 0 && *v <= 24)
        .ok_or("Invalid saved sheet.")?;
    let rows = count.div_ceil(cols);
    if cell as u64 >= cols * rows {
        return Err("Invalid saved frame cell.".into());
    }
    let png = base64::engine::general_purpose::STANDARD.encode(record.read()?);
    // The worker lays cells out before optional image downscaling. Reconstruct that canvas so odd edges
    // and duplicate requests preserve the exact cell boundary instead of dividing by the unique count.
    let edge = meta["sheet_edge"].as_u64().unwrap_or(w);
    if !(1..=1280).contains(&edge) {
        return Err("Invalid saved sheet edge.".into());
    }
    let original_w = edge / cols;
    let original_h = (edge / rows).min(original_w * 3 / 4).max(32);
    let (x, y, cw, ch) = if meta["output"] == "frame" {
        (0, 0, w, h)
    } else {
        let x = (cell as u64 % cols) * original_w * w / edge;
        let right = ((cell as u64 % cols) + 1) * original_w * w / edge;
        let y = (cell as u64 / cols) * original_h * h / (original_h * rows);
        let bottom = ((cell as u64 / cols) + 1) * original_h * h / (original_h * rows);
        (x, y, right - x, bottom - y)
    };
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{cw}" height="{ch}" viewBox="{x} {y} {cw} {ch}"><image width="{w}" height="{h}" href="data:image/png;base64,{png}"/></svg>"#
    );
    let time = frame["actual_ns"].as_u64().unwrap_or(0) / 1_000_000;
    let name = format!(
        "Frame at {}:{:02}.{:03}",
        time / 60000,
        time / 1000 % 60,
        time % 1000
    );
    Ok(FileContent {
        file: handle.into(),
        display: name.clone(),
        name,
        kind: FileKind::Image,
        size: record.bytes as u64,
        fingerprint: record.digest.clone(),
        text: None,
        truncated: false,
        data_url: Some(format!(
            "data:image/svg+xml;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(svg)
        )),
        too_large: false,
        open_class: "refused".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scene_metadata_never_offers_unsaved_image_handles() {
        let view = projection(
            &json!({"video_id":"metadata","planned_frames":24,"selected_frames":1,"image_count":0,"frames":[{"index":0,"frame_id":0,"actual_ns":1_000_000_000}]}),
        );
        assert_eq!(view["frames"], json!([]));
        assert_eq!(view["selected_frames"], 1);
    }
}
