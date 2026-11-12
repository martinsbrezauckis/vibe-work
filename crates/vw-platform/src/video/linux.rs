//! System `GStreamer` video-only pipeline. Explicit file source/demux/parser/decoder; no URI or audio source.
//! The helper supplies the sandbox/deadline; every job owns a fresh pipeline and bounded appsink.
use super::{Frame, Metadata, geometry};
use gst::prelude::*;
use gstreamer as gst;
use gstreamer_app::AppSink;
use gstreamer_video::VideoInfo;
use std::{
    io::Read,
    path::Path,
    sync::{Arc, Mutex},
};
use vw_video_types::{Code, Error};
mod selection;
#[derive(Default)]
struct Track {
    meta: Option<Metadata>,
    error: Option<Error>,
}
/// A fresh system decoder pipeline; dropping it closes every element.
pub struct Decoder {
    pipeline: gst::Pipeline,
    sink: AppSink,
    track: Arc<Mutex<Track>>,
    sequential: bool,
}
fn element(name: &str) -> Result<gst::Element, Error> {
    let media = match name {
        "qtdemux" => vw_video_types::Media::Mp4,
        "matroskademux" => vw_video_types::Media::Webm,
        _ => vw_video_types::Media::Gstreamer,
    };
    let factory = gst::ElementFactory::find(name).ok_or_else(|| {
        Error::new(
            Code::MediaUnavailable { media },
            format!("missing GStreamer element {name}"),
        )
    })?;
    factory
        .create()
        .build()
        .map_err(|error| Error::at(Code::MediaInitialization { media }, name, error))
}
fn lock(track: &Mutex<Track>) -> std::sync::MutexGuard<'_, Track> {
    track
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
impl Decoder {
    /// Opens a regular staged input, with metadata and source bounds checked before video decoding.
    /// # Errors
    /// Missing system media support, invalid media or an exceeded bound.
    #[allow(
        clippy::too_many_lines,
        reason = "pipeline elements and their dynamic-pad closure share one setup scope"
    )]
    pub fn open(path: &Path, edge: u32) -> Result<Self, Error> {
        if edge == 0 || edge > 4096 {
            return Err(Error::new(Code::ImageEdge, "output edge"));
        }
        let sequential = super::container::check(path)?;
        gst::init().map_err(|error| {
            Error::at(
                Code::MediaInitialization {
                    media: vw_video_types::Media::Gstreamer,
                },
                "GStreamer initialization",
                error,
            )
        })?;
        let mut file = std::fs::File::open(path)
            .map_err(|error| Error::at(Code::DecodeFailed, "input unavailable", error))?;
        if !file
            .metadata()
            .map_err(|error| Error::at(Code::DecodeFailed, "input metadata", error))?
            .is_file()
        {
            return Err(Error::new(Code::DecodeFailed, "regular file required"));
        }
        let mut magic = [0; 12];
        file.read_exact(&mut magic)
            .map_err(|error| Error::at(Code::DecodeFailed, "incomplete container header", error))?;
        let demux = if &magic[4..8] == b"ftyp" {
            "qtdemux"
        } else if magic[..4] == [0x1a, 0x45, 0xdf, 0xa3] {
            "matroskademux"
        } else {
            return Err(Error::new(
                Code::UnsupportedContainer,
                "MP4 or WebM required",
            ));
        };
        let pipeline = gst::Pipeline::new();
        let source = element("filesrc")?;
        source.set_property(
            "location",
            path.to_str()
                .ok_or(Error::new(Code::DecodeFailed, "invalid input path"))?,
        );
        let demux = element(demux)?;
        let sink = element("appsink")?.downcast::<AppSink>().map_err(|error| {
            Error::at(
                Code::MediaInitialization {
                    media: vw_video_types::Media::Gstreamer,
                },
                "appsink",
                error,
            )
        })?;
        sink.set_caps(Some(&gst::Caps::builder("video/x-raw").build()));
        sink.set_sync(false);
        sink.set_async(false);
        sink.set_max_buffers(1);
        sink.set_enable_last_sample(false);
        pipeline
            .add_many([&source, &demux, sink.upcast_ref()])
            .map_err(|error| Error::at(Code::DecodeFailed, "pipeline", error))?;
        source
            .link(&demux)
            .map_err(|error| Error::at(Code::DecodeFailed, "source link", error))?;
        let track = Arc::new(Mutex::new(Track::default()));
        let state = track.clone();
        let weak = pipeline.downgrade();
        let output = sink.clone();
        demux.connect_pad_added(move |_, pad| {
            let Some(caps) = pad.current_caps() else {
                return;
            };
            let Some(s) = caps.structure(0) else { return };
            if !s.name().starts_with("video/") || lock(&state).meta.is_some() {
                return;
            }
            let result = (|| {
                let meta = selection::metadata(s)?;
                let (parser, decoder) = selection::elements(s.name().as_str())?;
                let decoder = element(decoder)?;
                for prop in ["max-threads", "threads"] {
                    if let Some(p) = decoder.find_property(prop) {
                        if p.value_type() == u32::static_type() {
                            decoder.set_property(prop, 2u32);
                        } else {
                            decoder.set_property(prop, 2i32);
                        }
                    }
                }
                let parser = parser.map(element).transpose()?;
                let p = weak
                    .upgrade()
                    .ok_or(Error::new(Code::Cancelled, "cancelled"))?;
                p.add(&decoder)
                    .map_err(|error| Error::at(Code::DecodeFailed, "decoder add", error))?;
                decoder
                    .link(&output)
                    .map_err(|error| Error::at(Code::DecodeFailed, "decoder link", error))?;
                if let Some(parser) = parser {
                    p.add(&parser)
                        .map_err(|error| Error::at(Code::DecodeFailed, "parser add", error))?;
                    parser
                        .link(&decoder)
                        .map_err(|error| Error::at(Code::DecodeFailed, "parser link", error))?;
                    pad.link(
                        &parser
                            .static_pad("sink")
                            .ok_or(Error::new(Code::DecodeFailed, "parser pad"))?,
                    )
                    .map_err(|error| Error::at(Code::DecodeFailed, "video link", error))?;
                    parser
                        .sync_state_with_parent()
                        .map_err(|error| Error::at(Code::DecodeFailed, "parser state", error))?;
                } else {
                    pad.link(
                        &decoder
                            .static_pad("sink")
                            .ok_or(Error::new(Code::DecodeFailed, "decoder pad"))?,
                    )
                    .map_err(|error| Error::at(Code::DecodeFailed, "video link", error))?;
                }
                lock(&state).meta = Some(meta);
                decoder
                    .sync_state_with_parent()
                    .map_err(|error| Error::at(Code::DecodeFailed, "decoder state", error))?;
                Ok::<_, Error>(())
            })();
            if let Err(error) = result {
                lock(&state).error = Some(error);
            }
        });
        let mut d = Self {
            pipeline,
            sink,
            track,
            sequential,
        };
        d.pipeline
            .set_state(gst::State::Paused)
            .map_err(|error| Error::at(Code::DecodeFailed, "preroll", error))?;
        if d.sink
            .try_pull_preroll(gst::ClockTime::from_seconds(5))
            .is_none()
        {
            return Err(d.failure());
        }
        d.messages()?;
        let duration = d
            .pipeline
            .query_duration::<gst::ClockTime>()
            .ok_or(Error::new(Code::DecodeFailed, "unknown duration"))?
            .nseconds();
        if duration == 0 || duration > 7_200_000_000_000 {
            return Err(Error::new(Code::Duration, "video duration"));
        }
        lock(&d.track)
            .meta
            .as_mut()
            .ok_or(Error::new(Code::NoVideoTrack, "no video track"))?
            .duration_ns = duration;
        d.verify_end(duration)?;
        Ok(d)
    }
    /// Metadata from the selected video track.
    /// # Errors
    /// Invalid or missing track metadata.
    pub fn metadata(&self) -> Result<Metadata, Error> {
        self.messages()?;
        lock(&self.track)
            .meta
            .clone()
            .ok_or_else(|| Error::new(Code::NoVideoTrack, "no video track"))
    }
    fn verify_end(&mut self, duration: u64) -> Result<(), Error> {
        self.pipeline
            .seek_simple(
                gst::SeekFlags::FLUSH | gst::SeekFlags::KEY_UNIT | gst::SeekFlags::SNAP_BEFORE,
                gst::ClockTime::from_nseconds(if self.sequential { 0 } else { duration - 1 }),
            )
            .map_err(|error| Error::at(Code::DecodeFailed, "end-of-video seek", error))?;
        self.pipeline
            .set_state(gst::State::Playing)
            .map_err(|error| Error::at(Code::DecodeFailed, "end-of-video read", error))?;
        let mut end = None;
        let mut previous = None;
        for _ in 0..100_000 {
            let Some(sample) = self.sink.try_pull_sample(gst::ClockTime::from_seconds(5)) else {
                self.messages()?;
                if !self.sink.is_eos() {
                    return Err(self.failure());
                }
                let end =
                    end.ok_or(Error::new(Code::DecodeFailed, "incomplete video ending"))??;
                lock(&self.track)
                    .meta
                    .as_mut()
                    .ok_or(Error::new(Code::DecodeFailed, "metadata"))?
                    .duration_ns = end;
                self.pipeline
                    .set_state(gst::State::Paused)
                    .map_err(|error| Error::at(Code::DecodeFailed, "pause", error))?;
                return Ok(());
            };
            self.messages()?;
            let b = sample
                .buffer()
                .ok_or(Error::new(Code::DecodeFailed, "end buffer"))?;
            let segment = sample
                .segment()
                .and_then(|s| s.downcast_ref::<gst::ClockTime>())
                .ok_or(Error::new(Code::DecodeFailed, "end timeline"))?;
            let pts = segment
                .to_stream_time(b.pts().ok_or(Error::new(Code::DecodeFailed, "end PTS"))?)
                .ok_or(Error::new(Code::DecodeFailed, "end PTS"))?
                .nseconds();
            end = Some(
                frame_end(
                    pts,
                    b.duration().map(gst::ClockTime::nseconds),
                    previous,
                    duration,
                )
                .ok_or(Error::new(
                    Code::DecodeFailed,
                    "unknown final frame duration",
                )),
            );
            previous = Some(pts);
        }
        Err(Error::new(Code::DecodeLimit, "end-of-video sample count"))
    }
    fn messages(&self) -> Result<(), Error> {
        if let Some(bus) = self.pipeline.bus() {
            while let Some(msg) = bus.pop() {
                match msg.view() {
                    gst::MessageView::Error(error) => {
                        return Err(lock(&self.track).error.clone().unwrap_or_else(|| {
                            Error::at(
                                Code::DecodeFailed,
                                "GStreamer rejected the video",
                                error.error(),
                            )
                        }));
                    }
                    gst::MessageView::Tag(tags) => {
                        if let Some(tag) = tags.tags().get::<gst::tags::ImageOrientation>() {
                            let rotation = match tag.get() {
                                "rotate-90" => 90,
                                "rotate-180" => 180,
                                "rotate-270" => 270,
                                "rotate-0" => 0,
                                _ => {
                                    return Err(Error::new(
                                        Code::Rotation,
                                        "reflected video orientation",
                                    ));
                                }
                            };
                            if let Some(m) = lock(&self.track).meta.as_mut() {
                                m.rotation = rotation;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        if let Some(error) = &lock(&self.track).error {
            return Err(error.clone());
        }
        Ok(())
    }
    fn failure(&self) -> Error {
        self.messages().err().unwrap_or_else(|| {
            Error::new(
                Code::FrameMissing,
                "no complete frame before EOS or deadline",
            )
        })
    }
    /// Returns the frame covering the requested presentation time, never the first frame after it.
    /// # Errors
    /// EOF, missing frame, unsupported display conversion or failed decoding.
    pub fn frame(&mut self, ns: u64, edge: u32) -> Result<Frame, Error> {
        let meta = self.metadata()?;
        if ns >= meta.duration_ns {
            return Err(Error::new(
                Code::FrameMissing,
                "requested time is at or after EOF",
            ));
        }
        self.pipeline
            .set_state(gst::State::Paused)
            .map_err(|error| Error::at(Code::DecodeFailed, "pause", error))?;
        self.pipeline
            .seek_simple(
                gst::SeekFlags::FLUSH | gst::SeekFlags::KEY_UNIT | gst::SeekFlags::SNAP_BEFORE,
                // Fragmented MP4 seeking can clip PTS to the requested segment start. Starting
                // at zero preserves original PTS; the external deadline bounds long fragments.
                gst::ClockTime::from_nseconds(if self.sequential { 0 } else { ns }),
            )
            .map_err(|error| Error::at(Code::DecodeFailed, "video cannot seek", error))?;
        self.pipeline
            .set_state(gst::State::Playing)
            .map_err(|error| Error::at(Code::DecodeFailed, "play", error))?;
        let mut previous = None;
        let mut previous_pts = None;
        let mut end = 0;
        for _ in 0..100_000 {
            let Some(sample) = self.sink.try_pull_sample(gst::ClockTime::from_seconds(5)) else {
                self.messages()?;
                if self.sink.is_eos() && ns < end {
                    break;
                }
                return Err(self.failure());
            };
            self.messages()?;
            let pts = presentation_time(&sample)?;
            if pts > ns {
                break;
            }
            let duration = sample
                .buffer()
                .and_then(gst::BufferRef::duration)
                .map(gst::ClockTime::nseconds);
            end = frame_end(pts, duration, previous_pts, meta.duration_ns).unwrap_or(pts);
            previous_pts = Some(pts);
            previous = Some(sample);
        }
        self.pipeline
            .set_state(gst::State::Paused)
            .map_err(|error| Error::at(Code::DecodeFailed, "pause", error))?;
        geometry::display(
            &read(
                &previous.ok_or(Error::new(Code::FrameMissing, "no covering frame"))?,
                edge,
            )?,
            &self.metadata()?,
            edge,
        )
    }
}
impl Drop for Decoder {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}
/// End of a decoded frame on the stream timeline, used only to prove the final frame covers a
/// request. `GStreamer` 1.24's H.264 decoder leaves variable-frame-rate buffers without a duration;
/// like a player, the last frame is then held for the preceding presentation gap. `fallback` (the
/// container duration or the decoder's verified end) is used only when no earlier frame was seen
/// in this pass. Nothing here adds a frame: callers still return only a decoded sample.
fn frame_end(pts: u64, duration: Option<u64>, previous: Option<u64>, fallback: u64) -> Option<u64> {
    if let Some(duration) = duration {
        return pts.checked_add(duration);
    }
    match previous
        .and_then(|p| pts.checked_sub(p))
        .filter(|gap| *gap > 0)
    {
        Some(gap) => pts.checked_add(gap),
        None => (fallback > pts).then_some(fallback),
    }
}
fn presentation_time(sample: &gst::Sample) -> Result<u64, Error> {
    let buffer = sample
        .buffer()
        .ok_or(Error::new(Code::DecodeFailed, "no video buffer"))?;
    sample
        .segment()
        .and_then(|s| s.downcast_ref::<gst::ClockTime>())
        .ok_or(Error::new(Code::DecodeFailed, "invalid timeline"))?
        .to_stream_time(
            buffer
                .pts()
                .ok_or(Error::new(Code::DecodeFailed, "missing PTS"))?,
        )
        .map(gst::ClockTime::nseconds)
        .ok_or_else(|| Error::new(Code::DecodeFailed, "PTS outside timeline"))
}
fn read(sample: &gst::Sample, edge: u32) -> Result<Frame, Error> {
    use gstreamer_video::{VideoConverter, VideoFormat, VideoFrame};
    let info = VideoInfo::from_caps(
        sample
            .caps()
            .ok_or(Error::new(Code::DecodeFailed, "no frame caps"))?,
    )
    .map_err(|error| Error::at(Code::DecodeFailed, "invalid frame caps", error))?;
    geometry::geometry(info.width(), info.height())?;
    let divisor = info.width().max(info.height()).max(edge);
    let width =
        u32::try_from((u64::from(info.width()) * u64::from(edge) / u64::from(divisor)).max(1))
            .map_err(|error| Error::at(Code::Dimensions, "width", error))?;
    let height =
        u32::try_from((u64::from(info.height()) * u64::from(edge) / u64::from(divisor)).max(1))
            .map_err(|error| Error::at(Code::Dimensions, "height", error))?;
    let output = VideoInfo::builder(VideoFormat::Rgba, width, height)
        .fps(info.fps())
        .interlace_mode(info.interlace_mode())
        .build()
        .map_err(|error| Error::at(Code::DecodeFailed, "output layout", error))?;
    let converter = VideoConverter::new(&info, &output, None)
        .map_err(|error| Error::at(Code::DisplayConversion, "display conversion", error))?;
    let input = VideoFrame::from_buffer_readable(
        sample
            .buffer()
            .ok_or(Error::new(Code::DecodeFailed, "buffer"))?
            .to_owned(),
        &info,
    )
    .map_err(|error| Error::at(Code::DecodeFailed, "source layout", error))?;
    let buffer = gst::Buffer::with_size(output.size())
        .map_err(|error| Error::at(Code::Memory, "output buffer", error))?;
    let mut destination = VideoFrame::from_buffer_writable(buffer, &output)
        .map_err(|error| Error::at(Code::DecodeFailed, "output buffer", error))?;
    converter.frame(&input, &mut destination);
    let buffer = destination.into_buffer();
    let map = buffer
        .map_readable()
        .map_err(|error| Error::at(Code::DecodeFailed, "output read", error))?;
    let stride = usize::try_from(output.stride()[0])
        .map_err(|error| Error::at(Code::DecodeFailed, "stride", error))?;
    let row = width as usize * 4;
    let pixels = (0..height as usize)
        .flat_map(|y| map[y * stride..y * stride + row].iter().copied())
        .collect();
    Ok(Frame {
        pts_ns: presentation_time(sample)?,
        width,
        height,
        pixels,
    })
}
#[cfg(test)]
mod end_tests {
    use super::frame_end;
    #[test]
    fn final_frame_end_prefers_buffer_duration_then_last_gap_then_known_end() {
        // Buffer duration wins when GStreamer provides one.
        assert_eq!(frame_end(4_920, Some(80), Some(4_880), 4_940), Some(5_000));
        // h264-vfr.mp4 under GStreamer 1.24.2: no durations; last PTS gap 4.88 s -> 4.92 s.
        assert_eq!(
            frame_end(4_920_000_000, None, Some(4_880_000_000), 4_940_000_000),
            Some(4_960_000_000)
        );
        // A single frame in the pass uses the known end, never one at or before its own PTS.
        assert_eq!(frame_end(4_920, None, None, 4_940), Some(4_940));
        assert_eq!(frame_end(4_920, None, None, 4_920), None);
        assert_eq!(frame_end(4_920, None, Some(4_920), 4_000), None);
        assert_eq!(frame_end(u64::MAX, Some(1), None, 0), None);
    }
}
