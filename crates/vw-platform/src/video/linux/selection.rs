//! Maps container video caps to installed system elements, with precise missing-support errors.
use super::{Metadata, geometry, gst};
use vw_video_types::{Code, Error};
pub(super) fn metadata(s: &gst::StructureRef) -> Result<Metadata, Error> {
    let width = u32::try_from(
        s.get::<i32>("width")
            .map_err(|error| Error::at(Code::DecodeFailed, "unknown source width", error))?,
    )
    .map_err(|error| Error::at(Code::Dimensions, "width", error))?;
    let height = u32::try_from(
        s.get::<i32>("height")
            .map_err(|error| Error::at(Code::DecodeFailed, "unknown source height", error))?,
    )
    .map_err(|error| Error::at(Code::Dimensions, "height", error))?;
    geometry::geometry(width, height)?;
    if s.get::<String>("colorimetry").is_ok_and(|c| {
        c.contains("2100") || c.contains("2020") || c.contains("pq") || c.contains("hlg")
    }) {
        return Err(Error::new(Code::Hdr, "HDR color conversion is unavailable"));
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
            u32::try_from(par.numer())
                .map_err(|error| Error::at(Code::PixelAspect, "PAR", error))?,
            u32::try_from(par.denom())
                .map_err(|error| Error::at(Code::PixelAspect, "PAR", error))?,
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
pub(super) fn elements(codec: &str) -> Result<(Option<&'static str>, &'static str), Error> {
    choose(codec, |name| gst::ElementFactory::find(name).is_some())
}
fn choose(
    codec: &str,
    available: impl Fn(&str) -> bool,
) -> Result<(Option<&'static str>, &'static str), Error> {
    let (parser, decoders): (Option<&str>, &[&str]) = match codec {
        "video/x-h264" => (Some("h264parse"), &["avdec_h264", "openh264dec"]),
        "video/x-h265" => (Some("h265parse"), &["avdec_h265", "libde265dec"]),
        "video/x-vp8" => (None, &["vp8dec", "avdec_vp8"]),
        "video/x-vp9" => (None, &["vp9dec", "avdec_vp9"]),
        "video/x-av1" => (Some("av1parse"), &["dav1ddec", "av1dec", "avdec_av1"]),
        _ => return Err(Error::new(Code::UnsupportedCodec { codec: None }, codec)),
    };
    let known = match codec {
        "video/x-h264" => vw_video_types::Codec::H264,
        "video/x-h265" => vw_video_types::Codec::Hevc,
        "video/x-vp8" => vw_video_types::Codec::Vp8,
        "video/x-vp9" => vw_video_types::Codec::Vp9,
        "video/x-av1" => vw_video_types::Codec::Av1,
        _ => unreachable!("only supported caps reach element selection"),
    };
    let missing =
        |detail: String| Error::new(Code::UnsupportedCodec { codec: Some(known) }, detail);
    if let Some(parser) = parser
        && !available(parser)
    {
        return Err(missing(format!(
            "missing GStreamer element {parser} for {codec}"
        )));
    }
    let decoder = decoders.iter().find(|d| available(d)).ok_or_else(|| {
        missing(format!(
            "system has no decoder for {} ({codec}; {})",
            known.label(),
            decoders.join(" or ")
        ))
    })?;
    Ok((parser, *decoder))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn absent_system_support_names_the_codec_and_elements() {
        let parser = choose("video/x-h264", |_| false).unwrap_err();
        assert_eq!(
            parser.code,
            Code::UnsupportedCodec {
                codec: Some(vw_video_types::Codec::H264)
            }
        );
        assert!(parser.detail.contains("h264parse"));
        let vp9 = choose("video/x-vp9", |_| false).unwrap_err();
        assert_eq!(
            vp9.code,
            Code::UnsupportedCodec {
                codec: Some(vw_video_types::Codec::Vp9)
            }
        );
        assert!(vp9.detail.contains("vp9dec or avdec_vp9"));
        let h264 = choose("video/x-h264", |n| n == "h264parse").unwrap_err();
        assert_eq!(
            h264.code,
            Code::UnsupportedCodec {
                codec: Some(vw_video_types::Codec::H264)
            }
        );
        assert!(h264.detail.contains("avdec_h264 or openh264dec"));
        assert_eq!(
            choose("video/x-h264", |n| ["h264parse", "avdec_h264"].contains(&n))
                .unwrap()
                .1,
            "avdec_h264"
        );
    }
}
