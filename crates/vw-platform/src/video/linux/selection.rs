//! Maps container video caps to installed system elements, with precise missing-support errors.
use super::{Metadata, geometry, gst};
pub(super) fn metadata(s: &gst::StructureRef) -> Result<Metadata, String> {
    let width = u32::try_from(
        s.get::<i32>("width")
            .map_err(|_| "decode_failed: unknown source width")?,
    )
    .map_err(|_| "limit_exceeded: width")?;
    let height = u32::try_from(
        s.get::<i32>("height")
            .map_err(|_| "decode_failed: unknown source height")?,
    )
    .map_err(|_| "limit_exceeded: height")?;
    geometry::geometry(width, height)?;
    if s.get::<String>("colorimetry").is_ok_and(|c| {
        c.contains("2100") || c.contains("2020") || c.contains("pq") || c.contains("hlg")
    }) {
        return Err("unsupported_codec: HDR color conversion is unavailable".into());
    }
    let par = s
        .get::<gst::Fraction>("pixel-aspect-ratio")
        .unwrap_or(gst::Fraction::new(1, 1));
    Ok(Metadata {
        duration_ns: 0,
        width,
        height,
        rotation: 0,
        par: (
            u32::try_from(par.numer()).map_err(|_| "decode_failed: PAR")?,
            u32::try_from(par.denom()).map_err(|_| "decode_failed: PAR")?,
        ),
        codec: format!(
            "{} {}",
            s.name(),
            s.get::<String>("profile").unwrap_or_default()
        )
        .trim()
        .to_owned(),
    })
}
pub(super) fn elements(codec: &str) -> Result<(Option<&'static str>, &'static str), String> {
    choose(codec, |name| gst::ElementFactory::find(name).is_some())
}
fn choose(
    codec: &str,
    available: impl Fn(&str) -> bool,
) -> Result<(Option<&'static str>, &'static str), String> {
    let (parser, decoders): (Option<&str>, &[&str]) = match codec {
        "video/x-h264" => (Some("h264parse"), &["avdec_h264", "openh264dec"]),
        "video/x-h265" => (Some("h265parse"), &["avdec_h265", "libde265dec"]),
        "video/x-vp8" => (None, &["vp8dec", "avdec_vp8"]),
        "video/x-vp9" => (None, &["vp9dec", "avdec_vp9"]),
        "video/x-av1" => (Some("av1parse"), &["dav1ddec", "av1dec", "avdec_av1"]),
        _ => return Err(format!("unsupported_codec: {codec}")),
    };
    if let Some(parser) = parser
        && !available(parser)
    {
        return Err(format!(
            "backend_unavailable: missing GStreamer element {parser} for {codec}"
        ));
    }
    let label = match codec {
        "video/x-h264" => "H.264",
        "video/x-h265" => "HEVC",
        "video/x-vp8" => "VP8",
        "video/x-vp9" => "VP9",
        "video/x-av1" => "AV1",
        _ => codec,
    };
    let decoder = decoders.iter().find(|d| available(d)).ok_or_else(|| {
        format!(
            "unsupported_codec: system has no decoder for {label} ({codec}; {})",
            decoders.join(" or ")
        )
    })?;
    Ok((parser, *decoder))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn absent_system_support_names_the_codec_and_elements() {
        assert_eq!(
            choose("video/x-h264", |_| false).unwrap_err(),
            "backend_unavailable: missing GStreamer element h264parse for video/x-h264"
        );
        assert_eq!(
            choose("video/x-vp9", |_| false).unwrap_err(),
            "unsupported_codec: system has no decoder for VP9 (video/x-vp9; vp9dec or avdec_vp9)"
        );
        assert_eq!(
            choose("video/x-h264", |n| n == "h264parse").unwrap_err(),
            "unsupported_codec: system has no decoder for H.264 (video/x-h264; avdec_h264 or openh264dec)"
        );
        assert_eq!(
            choose("video/x-h264", |n| ["h264parse", "avdec_h264"].contains(&n))
                .unwrap()
                .1,
            "avdec_h264"
        );
    }
}
