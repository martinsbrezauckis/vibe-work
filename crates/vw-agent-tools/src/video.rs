//! Typed video requests shared by the MCP catalog and the optional core adapter.
//! Integer presentation times and preflight frame counts are validated before starting a decoder.
use serde::{Deserialize, Serialize};
pub use vw_video_types::{Code, Error};
/// The Windows spelling of a local video path. Other paths retain their spelling.
/// This lexical conversion grants no file access; callers still canonicalize and authorize the source.
#[must_use]
pub fn windows_path(path: &str) -> std::borrow::Cow<'_, str> {
    crate::msys_paths::native_drive(path)
        .map_or_else(|| std::borrow::Cow::Borrowed(path), std::borrow::Cow::Owned)
}
/// Shipping availability; Linux requires its explicit rollout feature.
pub const ENABLED: bool = cfg!(all(
    feature = "video",
    any(
        windows,
        target_os = "macos",
        all(target_os = "linux", feature = "linux-video")
    )
));
/// Reason and next action for a stale or direct call on an unsupported platform.
pub use vw_video_types::UNAVAILABLE;
/// Refuse before opening input, reserving images or starting a worker.
///
/// # Errors
/// The platform's reason and instruction when video review is unavailable.
pub fn require_available() -> Result<(), Error> {
    if ENABLED {
        Ok(())
    } else {
        Err(Error::new(
            Code::Unavailable,
            "video feature or platform unavailable",
        ))
    }
}
/// Check a named tool before forwarding it or consuming its saved approval.
///
/// # Errors
/// The platform refusal for an unavailable video call; other tools are unaffected.
pub fn check_tool(name: &str) -> Result<(), String> {
    if name == "video_frames" {
        require_available().map_err(|error| error.agent())?;
    }
    Ok(())
}
/// Maximum selected thumbnails in one bounded job (one MCP image).
pub const MAX_FRAMES: usize = 24;
/// Maximum selected range in milliseconds.
pub const MAX_RANGE_MS: u64 = 600_000;
/// Maximum small comparison frames scanned by a scene job.
pub const MAX_SCAN_SAMPLES: usize = 240;
/// Video request; output paths and decoder names are never model-controlled.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// Authorized local file.
    pub path: String,
    /// Selected presentation times.
    pub selection: Selection,
    /// One image or metadata only.
    pub output: Output,
    /// Optional smaller output edge; core applies the existing screenshot ceiling.
    pub max_edge: Option<u32>,
}
/// A single selection method.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Selection {
    /// Equal interval midpoints.
    Uniform {
        /// Range start.
        start_ms: u64,
        /// Exclusive range end.
        end_ms: u64,
        /// Planned thumbnails.
        count: usize,
    },
    /// Compare consecutive sampled pictures, returning the earliest detected changes up to the cap.
    SceneChanges {
        /// First comparison sample; a cut before this baseline is not detected.
        start_ms: u64,
        /// Exclusive range end.
        end_ms: u64,
        /// Fixed sampling step; events between samples may be missed.
        step_ms: u64,
        /// Maximum returned cut frames (not the number of comparison samples).
        max_frames: usize,
    },
    /// Explicit presentation times, preserving request indices.
    Timestamps {
        /// Milliseconds from the visible timeline start.
        times_ms: Vec<u64>,
    },
    /// Fixed spacing, with a refusal rather than silent resampling at the limit.
    Interval {
        /// Range start.
        start_ms: u64,
        /// Exclusive range end.
        end_ms: u64,
        /// Spacing.
        every_ms: u64,
        /// Caller may lower the frame cap.
        max_frames: usize,
    },
}
/// Image mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Output {
    /// One selected frame.
    Frame,
    /// One bounded composite.
    ContactSheet,
    /// No image.
    Metadata,
}
impl Request {
    /// Plans requested times (comparison samples for scenes) before any media is opened.
    /// # Errors
    /// Invalid selection or count/range limits.
    pub fn times(&self) -> Result<Vec<u64>, Error> {
        if self.max_edge.is_some_and(|edge| edge < 64) {
            return Err(Error::new(Code::ImageEdge, "image edge is below 64"));
        }
        let bad = || {
            Error::new(
                Code::Selection,
                "selection is outside the frame count or timeline limits",
            )
        };
        let range = |start: u64, end: u64| -> Result<u64, Error> {
            end.checked_sub(start)
                .filter(|n| *n > 0 && *n <= MAX_RANGE_MS && end <= 7_200_000)
                .ok_or_else(bad)
        };
        let times = match &self.selection {
            Selection::Uniform {
                start_ms,
                end_ms,
                count,
            } => {
                let span = range(*start_ms, *end_ms)?;
                if *count == 0 || *count > MAX_FRAMES {
                    return Err(bad());
                }
                (0..*count)
                    .map(|i| start_ms + span * (2 * i as u64 + 1) / (2 * (*count as u64)))
                    .collect()
            }
            Selection::SceneChanges {
                start_ms,
                end_ms,
                step_ms,
                max_frames,
            } => {
                let span = range(*start_ms, *end_ms)?;
                if *step_ms == 0 {
                    return Err(Error::new(Code::Step, "scene sampling step is zero"));
                }
                if !(1..=MAX_FRAMES).contains(max_frames) {
                    return Err(bad());
                }
                let count = span.div_ceil(*step_ms);
                if count > MAX_SCAN_SAMPLES as u64 {
                    return Err(Error::new(
                        Code::ScanSamples,
                        "scene scanning exceeds 240 samples",
                    ));
                }
                (0..count).map(|i| start_ms + i * step_ms).collect()
            }
            Selection::Timestamps { times_ms } => {
                if times_ms.is_empty() || times_ms.len() > MAX_FRAMES {
                    return Err(bad());
                }
                let min = times_ms.iter().min().copied().unwrap_or(0);
                let max = times_ms.iter().max().copied().unwrap_or(0);
                if max - min > MAX_RANGE_MS || max >= 7_200_000 {
                    return Err(bad());
                }
                times_ms.clone()
            }
            Selection::Interval {
                start_ms,
                end_ms,
                every_ms,
                max_frames,
            } => {
                let span = range(*start_ms, *end_ms)?;
                if *every_ms == 0 || !(1..=MAX_FRAMES).contains(max_frames) {
                    return Err(bad());
                }
                let count = span.div_ceil(*every_ms);
                if count > MAX_FRAMES as u64 || count > *max_frames as u64 {
                    return Err(bad());
                }
                (0..count).map(|i| start_ms + i * every_ms).collect()
            }
        };
        self.check_output(self.scene_limit().unwrap_or(times.len()))?;
        Ok(times)
    }
    fn check_output(&self, count: usize) -> Result<(), Error> {
        if self.output == Output::ContactSheet {
            let cols = count.min(4);
            let rows = count.div_ceil(cols);
            let minimum = (cols * 80).max(rows * 32);
            if self.max_edge.is_some_and(|edge| (edge as usize) < minimum) {
                return Err(Error::new(
                    Code::ImageEdge,
                    "image edge is too small for the frame count",
                ));
            }
        }
        if self.output == Output::Frame && count != 1 {
            return Err(Error::new(
                Code::FrameOutput,
                "single-frame output requested with multiple frames",
            ));
        }
        if self.path.is_empty() || self.path.len() > 4096 || self.path.contains('\0') {
            return Err(Error::new(
                Code::InvalidPath,
                "empty, NUL-containing or oversized local path",
            ));
        }
        Ok(())
    }
    /// The selected-frame upper bound; scene comparison samples do not consume image slots.
    /// # Errors
    /// Invalid request, including a scan that exceeds the work limit.
    pub fn frame_limit(&self) -> Result<usize, Error> {
        Ok(self.scene_limit().unwrap_or(self.times()?.len()))
    }
    /// Scene requests have an upper bound rather than an exact preflight count.
    #[must_use]
    pub fn scene_limit(&self) -> Option<usize> {
        match self.selection {
            Selection::SceneChanges { max_frames, .. } => Some(max_frames),
            _ => None,
        }
    }
    /// Human-readable preflight shared by the MCP stream and permission summary.
    /// # Errors
    /// Invalid selection.
    pub fn preflight(&self) -> Result<String, Error> {
        Ok(format!(
            "{}{} selected frames; {} image",
            if self.scene_limit().is_some() {
                "Up to "
            } else {
                ""
            },
            self.frame_limit()?,
            usize::from(self.output != Output::Metadata)
        ))
    }
    /// Checks declared endpoints, including an end beyond EOF whose samples would fit.
    /// # Errors
    /// A selection lies outside the visible duration. The end is exclusive and may equal duration.
    pub fn validate_duration(&self, duration_ns: u64) -> Result<(), Error> {
        let times = self.times()?;
        let outside = match self.selection {
            Selection::Uniform {
                start_ms, end_ms, ..
            }
            | Selection::Interval {
                start_ms, end_ms, ..
            }
            | Selection::SceneChanges {
                start_ms, end_ms, ..
            } => start_ms * 1_000_000 >= duration_ns || end_ms * 1_000_000 > duration_ns,
            Selection::Timestamps { .. } => times.iter().any(|ms| ms * 1_000_000 >= duration_ns),
        };
        if outside {
            return Err(Error::new(
                Code::OutsideDuration { duration_ns },
                "requested selection extends beyond the visible duration",
            ));
        }
        Ok(())
    }
}

#[cfg(feature = "video")]
pub(crate) fn spec() -> serde_json::Value {
    use serde_json::json;
    let integer = json!({"type":"integer","minimum":0,"maximum":7_200_000});
    let range = |kind: &str, extra: &str| json!({"type":"object","additionalProperties":false,"required":["kind","start_ms","end_ms",extra],"properties":{"kind":{"const":kind},"start_ms":integer,"end_ms":integer}});
    let mut uniform = range("uniform", "count");
    uniform["properties"]["count"] = json!({"type":"integer","minimum":1,"maximum":MAX_FRAMES});
    let mut interval = range("interval", "every_ms");
    interval["required"] = json!(["kind", "start_ms", "end_ms", "every_ms", "max_frames"]);
    interval["properties"]["every_ms"] =
        json!({"type":"integer","minimum":1,"maximum":MAX_RANGE_MS});
    interval["properties"]["max_frames"] =
        json!({"type":"integer","minimum":1,"maximum":MAX_FRAMES});
    let mut scenes = range("scene_changes", "step_ms");
    scenes["required"] = json!(["kind", "start_ms", "end_ms", "step_ms", "max_frames"]);
    scenes["properties"]["step_ms"] = json!({"type":"integer","minimum":1,"maximum":MAX_RANGE_MS});
    scenes["properties"]["max_frames"] = json!({"type":"integer","minimum":1,"maximum":MAX_FRAMES});
    let platform = if cfg!(all(target_os = "linux", feature = "linux-video")) {
        "Linux: extract selected video moments from a regular local file up to 512 MiB and at most two hours long using this computer's installed media support."
    } else {
        "Windows and macOS only: extract selected video moments from a regular local file up to 512 MiB and at most two hours long using Windows Media Foundation or macOS AVFoundation."
    };
    let description = format!(
        "{platform} Select at most 24 frames across ten minutes; frame output requires one frame. Images share this user turn's image allowance. Returns a single frame or contact sheet as image content with requested and actual presentation times; metadata output uses the same media support and sends no image. Reports the planned frame and image counts before extraction. Uniform gives an overview; uniform, interval and scene_changes use start_ms and exclusive end_ms, refused outside the video. Scene changes compare consecutive small pictures at step_ms (at most 240 samples), returning the earliest cuts up to max_frames and reporting sampled coverage and omitted cuts. Subtle cuts and events between samples may be missed; request a narrower range or smaller step for more detail. Audio is not processed. Unsupported formats or codecs are reported separately from worker failures and limits."
    );
    json!({"name":"video_frames","description":description,"annotations":{"readOnlyHint":true},"inputSchema":{"type":"object","additionalProperties":false,"required":["path","selection","output"],"properties":{
        "path":{"type":"string","minLength":1,"maxLength":4096},
        "selection":{"oneOf":[uniform,interval,scenes,{"type":"object","additionalProperties":false,"required":["kind","times_ms"],"properties":{"kind":{"const":"timestamps"},"times_ms":{"type":"array","minItems":1,"maxItems":MAX_FRAMES,"items":integer}}}]},
        "output":{"enum":["frame","contact_sheet","metadata"]},"max_edge":{"type":"integer","minimum":64,"maximum":1280}
    }}})
}

#[cfg(feature = "video")]
pub(crate) fn progress(message: &serde_json::Value) -> Option<serde_json::Value> {
    use serde_json::json;
    if !ENABLED || message["method"] != "tools/call" || message["params"]["name"] != "video_frames"
    {
        return None;
    }
    let mut args = message["params"]["arguments"].clone();
    args.as_object_mut()?
        .remove(crate::wire::tools::RECEIPT_FIELD);
    let request: Request = serde_json::from_value(args).ok()?;
    let frames = request.frame_limit().ok()?;
    let images = usize::from(request.output != Output::Metadata);
    let text = request.preflight().ok()?;
    let token = &message["params"]["_meta"]["progressToken"];
    Some(if token.is_string() || token.is_number() {
        json!({"jsonrpc":"2.0","method":"notifications/progress","params":{"progressToken":token,"progress":0,"total":frames,"message":text}})
    } else {
        json!({"jsonrpc":"2.0","method":"notifications/message","params":{"level":"info","logger":"video_frames","data":{"message":text,"frame_count":frames,"image_count":images}}})
    })
}
