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
mod selection;
#[derive(Default)]
struct Track {
    meta: Option<Metadata>,
    error: Option<String>,
}
/// A fresh system decoder pipeline; dropping it closes every element.
pub struct Decoder {
    pipeline: gst::Pipeline,
    sink: AppSink,
    track: Arc<Mutex<Track>>,
    sequential: bool,
}
fn element(name: &str) -> Result<gst::Element, String> {
    gst::ElementFactory::make(name)
        .build()
        .map_err(|_| format!("backend_unavailable: missing GStreamer element {name}"))
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
    pub fn open(path: &Path, edge: u32) -> Result<Self, String> {
        if edge == 0 || edge > 4096 {
            return Err("limit_exceeded: output edge".into());
        }
        let sequential = super::container::check(path)?;
        gst::init().map_err(|_| "backend_unavailable: GStreamer initialization")?;
        let mut file = std::fs::File::open(path).map_err(|_| "decode_failed: input unavailable")?;
        if !file
            .metadata()
            .map_err(|_| "decode_failed: input metadata")?
            .is_file()
        {
            return Err("decode_failed: regular file required".into());
        }
        let mut magic = [0; 12];
        file.read_exact(&mut magic)
            .map_err(|_| "decode_failed: incomplete container header")?;
        let demux = if &magic[4..8] == b"ftyp" {
            "qtdemux"
        } else if magic[..4] == [0x1a, 0x45, 0xdf, 0xa3] {
            "matroskademux"
        } else {
            return Err("unsupported_container: MP4 or WebM required".into());
        };
        let pipeline = gst::Pipeline::new();
        let source = element("filesrc")?;
        source.set_property(
            "location",
            path.to_str().ok_or("decode_failed: invalid input path")?,
        );
        let demux = element(demux)?;
        let sink = element("appsink")?
            .downcast::<AppSink>()
            .map_err(|_| "backend_unavailable: appsink")?;
        sink.set_caps(Some(&gst::Caps::builder("video/x-raw").build()));
        sink.set_sync(false);
        sink.set_async(false);
        sink.set_max_buffers(1);
        sink.set_enable_last_sample(false);
        pipeline
            .add_many([&source, &demux, sink.upcast_ref()])
            .map_err(|_| "decode_failed: pipeline")?;
        source
            .link(&demux)
            .map_err(|_| "decode_failed: source link")?;
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
                let p = weak.upgrade().ok_or("cancelled")?;
                p.add(&decoder).map_err(|_| "decode_failed: decoder add")?;
                decoder
                    .link(&output)
                    .map_err(|_| "decode_failed: decoder link")?;
                if let Some(parser) = parser {
                    p.add(&parser).map_err(|_| "decode_failed: parser add")?;
                    parser
                        .link(&decoder)
                        .map_err(|_| "decode_failed: parser link")?;
                    pad.link(
                        &parser
                            .static_pad("sink")
                            .ok_or("decode_failed: parser pad")?,
                    )
                    .map_err(|_| "decode_failed: video link")?;
                    parser
                        .sync_state_with_parent()
                        .map_err(|_| "decode_failed: parser state")?;
                } else {
                    pad.link(
                        &decoder
                            .static_pad("sink")
                            .ok_or("decode_failed: decoder pad")?,
                    )
                    .map_err(|_| "decode_failed: video link")?;
                }
                lock(&state).meta = Some(meta);
                decoder
                    .sync_state_with_parent()
                    .map_err(|_| "decode_failed: decoder state")?;
                Ok::<_, String>(())
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
            .map_err(|_| "decode_failed: preroll")?;
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
            .ok_or("decode_failed: unknown duration")?
            .nseconds();
        if duration == 0 || duration > 7_200_000_000_000 {
            return Err("limit_exceeded: video duration".into());
        }
        lock(&d.track)
            .meta
            .as_mut()
            .ok_or("unsupported_codec: no video track")?
            .duration_ns = duration;
        d.verify_end(duration)?;
        Ok(d)
    }
    /// Metadata from the selected video track.
    /// # Errors
    /// Invalid or missing track metadata.
    pub fn metadata(&self) -> Result<Metadata, String> {
        self.messages()?;
        lock(&self.track)
            .meta
            .clone()
            .ok_or_else(|| "unsupported_codec: no video track".into())
    }
    fn verify_end(&mut self, duration: u64) -> Result<(), String> {
        self.pipeline
            .seek_simple(
                gst::SeekFlags::FLUSH | gst::SeekFlags::KEY_UNIT | gst::SeekFlags::SNAP_BEFORE,
                gst::ClockTime::from_nseconds(if self.sequential { 0 } else { duration - 1 }),
            )
            .map_err(|_| "decode_failed: end-of-video seek")?;
        self.pipeline
            .set_state(gst::State::Playing)
            .map_err(|_| "decode_failed: end-of-video read")?;
        let mut end = None;
        for _ in 0..100_000 {
            let Some(sample) = self.sink.try_pull_sample(gst::ClockTime::from_seconds(5)) else {
                self.messages()?;
                if !self.sink.is_eos() {
                    return Err(self.failure());
                }
                let end = end.ok_or("decode_failed: incomplete video ending")?;
                lock(&self.track)
                    .meta
                    .as_mut()
                    .ok_or("decode_failed: metadata")?
                    .duration_ns = end;
                self.pipeline
                    .set_state(gst::State::Paused)
                    .map_err(|_| "decode_failed: pause")?;
                return Ok(());
            };
            self.messages()?;
            let b = sample.buffer().ok_or("decode_failed: end buffer")?;
            let segment = sample
                .segment()
                .and_then(|s| s.downcast_ref::<gst::ClockTime>())
                .ok_or("decode_failed: end timeline")?;
            let pts = segment
                .to_stream_time(b.pts().ok_or("decode_failed: end PTS")?)
                .ok_or("decode_failed: end PTS")?
                .nseconds();
            end = pts.checked_add(
                b.duration()
                    .ok_or("decode_failed: unknown final frame duration")?
                    .nseconds(),
            );
        }
        Err("limit_exceeded: end-of-video sample count".into())
    }
    fn messages(&self) -> Result<(), String> {
        if let Some(bus) = self.pipeline.bus() {
            while let Some(msg) = bus.pop() {
                match msg.view() {
                    gst::MessageView::Error(_) => {
                        return Err(lock(&self.track).error.clone().unwrap_or_else(|| {
                            "decode_failed: GStreamer rejected the video".into()
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
                                    return Err(
                                        "unsupported_codec: reflected video orientation".into()
                                    );
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
    fn failure(&self) -> String {
        self.messages()
            .err()
            .unwrap_or_else(|| "decode_failed: no complete frame before EOS or deadline".into())
    }
    /// Returns the frame covering the requested presentation time, never the first frame after it.
    /// # Errors
    /// EOF, missing frame, unsupported display conversion or failed decoding.
    pub fn frame(&mut self, ns: u64, edge: u32) -> Result<Frame, String> {
        let meta = self.metadata()?;
        if ns >= meta.duration_ns {
            return Err("decode_failed: requested time is at or after EOF".into());
        }
        self.pipeline
            .set_state(gst::State::Paused)
            .map_err(|_| "decode_failed: pause")?;
        self.pipeline
            .seek_simple(
                gst::SeekFlags::FLUSH | gst::SeekFlags::KEY_UNIT | gst::SeekFlags::SNAP_BEFORE,
                // Fragmented MP4 seeking can clip PTS to the requested segment start. Starting
                // at zero preserves original PTS; the external deadline bounds long fragments.
                gst::ClockTime::from_nseconds(if self.sequential { 0 } else { ns }),
            )
            .map_err(|_| "decode_failed: video cannot seek")?;
        self.pipeline
            .set_state(gst::State::Playing)
            .map_err(|_| "decode_failed: play")?;
        let mut previous = None;
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
                .map_or(0, gst::ClockTime::nseconds);
            end = pts.saturating_add(duration);
            previous = Some(sample);
        }
        self.pipeline
            .set_state(gst::State::Paused)
            .map_err(|_| "decode_failed: pause")?;
        geometry::display(
            &read(&previous.ok_or("decode_failed: no covering frame")?, edge)?,
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
fn presentation_time(sample: &gst::Sample) -> Result<u64, String> {
    let buffer = sample.buffer().ok_or("decode_failed: no video buffer")?;
    sample
        .segment()
        .and_then(|s| s.downcast_ref::<gst::ClockTime>())
        .ok_or("decode_failed: invalid timeline")?
        .to_stream_time(buffer.pts().ok_or("decode_failed: missing PTS")?)
        .map(gst::ClockTime::nseconds)
        .ok_or_else(|| "decode_failed: PTS outside timeline".into())
}
fn read(sample: &gst::Sample, edge: u32) -> Result<Frame, String> {
    use gstreamer_video::{VideoConverter, VideoFormat, VideoFrame};
    let info = VideoInfo::from_caps(sample.caps().ok_or("decode_failed: no frame caps")?)
        .map_err(|_| "decode_failed: invalid frame caps")?;
    geometry::geometry(info.width(), info.height())?;
    let divisor = info.width().max(info.height()).max(edge);
    let width =
        u32::try_from((u64::from(info.width()) * u64::from(edge) / u64::from(divisor)).max(1))
            .map_err(|_| "limit_exceeded: width")?;
    let height =
        u32::try_from((u64::from(info.height()) * u64::from(edge) / u64::from(divisor)).max(1))
            .map_err(|_| "limit_exceeded: height")?;
    let output = VideoInfo::builder(VideoFormat::Rgba, width, height)
        .fps(info.fps())
        .interlace_mode(info.interlace_mode())
        .build()
        .map_err(|_| "decode_failed: output layout")?;
    let converter = VideoConverter::new(&info, &output, None)
        .map_err(|_| "unsupported_codec: display conversion")?;
    let input = VideoFrame::from_buffer_readable(
        sample.buffer().ok_or("decode_failed: buffer")?.to_owned(),
        &info,
    )
    .map_err(|_| "decode_failed: source layout")?;
    let buffer =
        gst::Buffer::with_size(output.size()).map_err(|_| "limit_exceeded: output buffer")?;
    let mut destination = VideoFrame::from_buffer_writable(buffer, &output)
        .map_err(|_| "decode_failed: output buffer")?;
    converter.frame(&input, &mut destination);
    let buffer = destination.into_buffer();
    let map = buffer
        .map_readable()
        .map_err(|_| "decode_failed: output read")?;
    let stride = usize::try_from(output.stride()[0]).map_err(|_| "decode_failed: stride")?;
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
