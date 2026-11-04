//! Narrow helper protocol. The parent validates counts, geometry, timestamps, image magic and job identity.
use serde::{Deserialize, Serialize};
use vw_agent_tools::video::Request;
/// Immutable request sent only to the constrained helper.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Job {
    /// Random per-extraction identity.
    pub id: String,
    /// Model request with the private staged input substituted for its path.
    pub request: Request,
}
/// Metadata for every requested time, including deduplicated frame identity.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Frame {
    /// Original request index.
    pub index: usize,
    /// Requested time.
    pub requested_ms: u64,
    /// Actual normalized presentation timestamp, without millisecond rounding.
    pub actual_ns: u64,
    /// Index of the unique returned frame.
    pub frame_id: usize,
}
/// Complete extraction; partial results never use this shape.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reply {
    /// Job identity echoed by the worker.
    pub id: String,
    /// Native API name and version.
    pub backend: String,
    /// Visible duration.
    pub duration_ns: u64,
    /// Source dimensions before display transforms.
    pub source_size: [u32; 2],
    /// Applied clockwise display rotation.
    pub rotation: u32,
    /// Encoded pixel aspect ratio.
    pub pixel_aspect_ratio: [u32; 2],
    /// Selected track codec/profile.
    pub codec: String,
    /// One row per requested time.
    pub frames: Vec<Frame>,
    /// Number of planned frame requests, before deduplication.
    pub planned_frames: usize,
    /// Completed sampled scene coverage; absent for fixed selections.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scan: Option<crate::scenes::Scan>,
    /// Number of image blocks.
    pub image_count: usize,
    /// Output size, zero for metadata.
    pub image_size: [u32; 2],
    /// Sheet columns, zero for metadata.
    pub columns: u32,
    /// Bounded image content; never an output path.
    pub png: String,
}
/// Validates an untrusted worker reply without decoding images in the parent.
/// # Errors
/// Any mismatch or cap violation.
#[allow(
    clippy::too_many_lines,
    reason = "one bounded reply validator keeps all envelope invariants together"
)]
pub fn validate(job: &Job, reply: &Reply) -> Result<(), String> {
    use base64::Engine as _;
    use vw_agent_tools::video::Output;
    let invalid = || "decode_failed: invalid or incomplete helper reply".to_owned();
    let times = job.request.times()?;
    if reply.id != job.id
        || reply.planned_frames != job.request.frame_limit()?
        || reply.duration_ns == 0
        || reply.duration_ns > 7_200_000_000_000
        || !matches!(reply.rotation, 0 | 90 | 180 | 270)
        || reply.codec.len() > 128
        || reply.backend.len() > 128
    {
        return Err(invalid());
    }
    let [w, h] = reply.source_size;
    if w == 0
        || h == 0
        || w > 4096
        || h > 4096
        || u64::from(w) * u64::from(h) > 8_847_360
        || reply.pixel_aspect_ratio.contains(&0)
    {
        return Err(invalid());
    }
    job.request.validate_duration(reply.duration_ns)?;
    let count = match (&reply.scan, job.request.scene_limit()) {
        (Some(scan), Some(_)) => scan.validate(&job.request)?,
        (None, None) => {
            if job.request.output == Output::Metadata {
                0
            } else {
                times.len()
            }
        }
        _ => return Err(invalid()),
    };
    if reply.frames.len() != count {
        return Err(invalid());
    }
    let mut pts = Vec::new();
    let mut last_request = None;
    for (i, frame) in reply.frames.iter().enumerate() {
        let ms = frame.requested_ms;
        let valid_time = if reply.scan.is_some() {
            times.binary_search(&ms).is_ok_and(|index| index > 0)
                && last_request.is_none_or(|last| ms > last)
                && pts.last().is_none_or(|last| frame.actual_ns > *last)
        } else {
            times.get(i) == Some(&ms)
        };
        if frame.index != i
            || !valid_time
            || frame.actual_ns > ms * 1_000_000
            || frame.actual_ns >= reply.duration_ns
        {
            return Err(invalid());
        }
        last_request = Some(ms);
        if frame.frame_id == pts.len() {
            pts.push(frame.actual_ns);
        } else if pts.get(frame.frame_id) != Some(&frame.actual_ns) {
            return Err(invalid());
        }
    }
    if job.request.output == Output::Metadata || count == 0 {
        return if reply.png.is_empty()
            && reply.image_count == 0
            && reply.image_size == [0, 0]
            && reply.columns == 0
        {
            Ok(())
        } else {
            Err(invalid())
        };
    }
    if reply.image_count != 1 || reply.columns as usize != count.min(4) {
        return Err(invalid());
    }
    let edge = job
        .request
        .max_edge
        .unwrap_or(vw_preview::MAX_WIDTH)
        .min(vw_preview::MAX_WIDTH);
    let [w, h] = reply.image_size;
    if w == 0
        || h == 0
        || w > edge
        || h > edge
        || reply.png.len() > vw_preview::MAX_BYTES.div_ceil(3) * 4
    {
        return Err(invalid());
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&reply.png)
        .map_err(|_| invalid())?;
    if bytes.len() > vw_preview::MAX_BYTES
        || bytes.len() < 45
        || bytes[..8] != [137, 80, 78, 71, 13, 10, 26, 10]
        || &bytes[12..16] != b"IHDR"
        || bytes[16..20] != w.to_be_bytes()
        || bytes[20..24] != h.to_be_bytes()
        || bytes[24] != 8
        || ![2, 6].contains(&bytes[25])
    {
        return Err(invalid());
    }
    let mut at = 8;
    let mut data = false;
    while at + 12 <= bytes.len() {
        let n = u32::from_be_bytes(bytes[at..at + 4].try_into().map_err(|_| invalid())?) as usize;
        let end = at
            .checked_add(12)
            .and_then(|v| v.checked_add(n))
            .filter(|v| *v <= bytes.len())
            .ok_or_else(invalid)?;
        let kind = &bytes[at + 4..at + 8];
        if kind == b"IDAT" {
            data = true;
        }
        if kind == b"IEND" {
            return if n == 0 && end == bytes.len() && data {
                Ok(())
            } else {
                Err(invalid())
            };
        }
        at = end;
    }
    Err(invalid())
}

impl Reply {
    /// Converts validated worker output to the existing image envelope; image bytes stay out of text metadata.
    #[must_use]
    pub fn into_tool(self) -> serde_json::Value {
        use serde_json::json;
        let selected = if self.scan.is_some() || self.image_count > 0 {
            self.frames.len()
        } else {
            self.planned_frames
        };
        let mut coverage = self.scan.as_ref().map_or_else(|| format!("{} selected moments; events between them may be absent. Request a narrower range or explicit timestamps to inspect further. Actual presentation times identify the covering frames and may precede the requested start.", self.planned_frames), crate::scenes::Scan::coverage);
        if self.image_count == 0 {
            coverage = format!(
                "No video image was sent. Request explicit timestamps with image output to view pictures. {coverage}"
            );
        }
        let mut value = json!({"status":"ready","video_id":self.id,"backend":self.backend,"duration_ns":self.duration_ns,"source_size":self.source_size,"rotation_applied":self.rotation,"pixel_aspect_ratio":self.pixel_aspect_ratio,"codec":self.codec,"frames":self.frames,"planned_frames":self.planned_frames,"image_count":self.image_count,"image_size":self.image_size,"columns":self.columns,"audio":"not_processed","coverage":coverage,"scan":self.scan,"selected_frames":selected,"color":"8-bit display conversion; exact HDR and 10-bit color is not preserved","model_context":"remaining image context is not available; conservative image limits applied"});
        if self.image_count == 1 {
            value[vw_agent_tools::wire::IMAGE_KEY] =
                json!({"mimeType":"image/png","data":self.png});
        }
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pair() -> (Job, Reply) {
        let job=Job{id:"one".into(),request:serde_json::from_value(serde_json::json!({"path":"input","selection":{"kind":"timestamps","times_ms":[999]},"output":"frame"})).unwrap()};
        let r = Reply {
            id: "other".into(),
            backend: "gstreamer".into(),
            duration_ns: 4_000_000_000,
            source_size: [640, 360],
            rotation: 0,
            pixel_aspect_ratio: [1, 1],
            codec: "h264".into(),
            frames: vec![],
            planned_frames: 1,
            scan: None,
            image_count: 1,
            image_size: [640, 360],
            columns: 1,
            png: "invalid".into(),
        };
        (job, r)
    }
    #[test]
    fn malformed_or_partial_output_is_never_ready() {
        let (j, r) = pair();
        assert!(validate(&j, &r).is_err());
    }
    #[test]
    fn helper_bytes_and_pixels_cannot_raise_the_image_budget() {
        let (j, mut r) = pair();
        r.id = j.id.clone();
        r.png = "A".repeat(vw_preview::MAX_BYTES * 2);
        assert!(validate(&j, &r).is_err());
        r.png.clear();
        r.image_size = [u32::MAX, u32::MAX];
        assert!(validate(&j, &r).is_err());
    }
}
