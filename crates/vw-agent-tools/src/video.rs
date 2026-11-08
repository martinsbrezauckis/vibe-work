//! Typed video requests shared by the MCP catalog and the optional core adapter.
//! Integer presentation times and preflight frame counts are validated before starting a decoder.
use serde::{Deserialize, Serialize};
/// The Windows spelling of a local video path. Other paths retain their spelling.
/// This lexical conversion grants no file access; callers still canonicalize and authorize the source.
#[must_use]
pub fn windows_path(path: &str) -> std::borrow::Cow<'_, str> {
    crate::msys_paths::native_drive(path)
        .map_or_else(|| std::borrow::Cow::Borrowed(path), std::borrow::Cow::Owned)
}
/// Shipping availability; keep the Linux implementation compiled for its future release.
pub const ENABLED: bool = cfg!(all(feature = "video", any(windows, target_os = "macos")));
/// Reason and next action for a stale or direct call on an unsupported platform.
pub const UNAVAILABLE: &str = if cfg!(target_os = "linux") {
    "Not run: video review is not available on Linux yet. Ask for still images or review the video on Windows or macOS."
} else {
    "Not run: video review is not available in this build. Ask for still images."
};
/// Refuse before opening input, reserving images or starting a worker.
///
/// # Errors
/// The platform's reason and instruction when video review is unavailable.
pub fn require_available() -> Result<(), String> {
    if ENABLED {
        Ok(())
    } else {
        Err(UNAVAILABLE.into())
    }
}
/// Check a named tool before forwarding it or consuming its saved approval.
///
/// # Errors
/// The platform refusal for an unavailable video call; other tools are unaffected.
pub fn check_tool(name: &str) -> Result<(), String> {
    if name == "video_frames" {
        require_available()?;
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
    pub fn times(&self) -> Result<Vec<u64>, String> {
        if self.max_edge.is_some_and(|edge| edge < 64) {
            return Err(
                "limit_exceeded: image edge must be at least 64. Choose a larger max_edge.".into(),
            );
        }
        let bad = || {
            "limit_exceeded: select 1–24 frames over a positive range of at most ten minutes within the first two hours. Choose a smaller valid selection.".to_owned()
        };
        let range = |start: u64, end: u64| -> Result<u64, String> {
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
                    return Err("limit_exceeded: step_ms must be positive. Set step_ms to at least 1 millisecond.".into());
                }
                if !(1..=MAX_FRAMES).contains(max_frames) {
                    return Err(bad());
                }
                let count = span.div_ceil(*step_ms);
                if count > MAX_SCAN_SAMPLES as u64 {
                    return Err("limit_exceeded: scene scanning allows at most 240 samples. Increase step_ms or choose a shorter range.".into());
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
        let count = self.scene_limit().unwrap_or(times.len());
        if self.output == Output::ContactSheet {
            let cols = count.min(4);
            let rows = count.div_ceil(cols);
            let minimum = (cols * 80).max(rows * 32);
            if self.max_edge.is_some_and(|edge| (edge as usize) < minimum) {
                return Err(
                    "limit_exceeded: image edge is too small for the selected frame count. Increase max_edge or reduce the frame count.".into(),
                );
            }
        }
        if self.output == Output::Frame && count != 1 {
            return Err("limit_exceeded: frame output requires a one-frame selection or scene cap. Choose contact_sheet output or set the frame count to one.".into());
        }
        if self.path.is_empty() || self.path.len() > 4096 || self.path.contains('\0') {
            return Err("decode_failed: invalid local path. Provide a nonempty local video path without NUL bytes, at most 4096 bytes long.".into());
        }
        Ok(times)
    }
    /// The selected-frame upper bound; scene comparison samples do not consume image slots.
    /// # Errors
    /// Invalid request, including a scan that exceeds the work limit.
    pub fn frame_limit(&self) -> Result<usize, String> {
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
    pub fn preflight(&self) -> Result<String, String> {
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
    pub fn validate_duration(&self, duration_ns: u64) -> Result<(), String> {
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
            return Err(format!(
                "range_outside_video: requested selection exceeds the video duration of {duration_ns} ns ({} ms). Choose start_ms before the duration and end_ms no later than it; explicit timestamps must be before the duration.",
                duration_ns / 1_000_000
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
    json!({"name":"video_frames","description":"Windows and macOS only: extract selected video moments from a regular local file up to 512 MiB and at most two hours long using Windows Media Foundation or macOS AVFoundation. Select at most 24 frames across ten minutes; frame output requires one frame. Images share this user turn's image allowance. Returns a single frame or contact sheet as image content with requested and actual presentation times; metadata output uses the same media support and sends no image. Reports the planned frame and image counts before extraction. Uniform gives an overview; uniform, interval and scene_changes use start_ms and exclusive end_ms, refused outside the video. Scene changes compare consecutive small pictures at step_ms (at most 240 samples), returning the earliest cuts up to max_frames and reporting sampled coverage and omitted cuts. Subtle cuts and events between samples may be missed; request a narrower range or smaller step for more detail. Audio is not processed. Unsupported formats or codecs are reported separately from worker failures and limits.","annotations":{"readOnlyHint":true},"inputSchema":{"type":"object","additionalProperties":false,"required":["path","selection","output"],"properties":{
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

#[cfg(test)]
mod tests {
    use super::*;
    fn request(s: Selection) -> Request {
        Request {
            path: "clip.mp4".into(),
            selection: s,
            output: Output::ContactSheet,
            max_edge: None,
        }
    }
    #[test]
    fn counts_and_midpoints_are_known_before_decode() {
        assert_eq!(
            request(Selection::Uniform {
                start_ms: 0,
                end_ms: 4000,
                count: 4
            })
            .times()
            .unwrap(),
            vec![500, 1500, 2500, 3500]
        );
        assert_eq!(
            request(Selection::Interval {
                start_ms: 0,
                end_ms: 4000,
                every_ms: 1500,
                max_frames: 4
            })
            .times()
            .unwrap(),
            vec![0, 1500, 3000]
        );
    }
    #[test]
    fn selection_errors_keep_the_diagnosis_and_repair_instruction() {
        let mut r = request(Selection::SceneChanges {
            start_ms: 0,
            end_ms: 4000,
            step_ms: 0,
            max_frames: 24,
        });
        let error = r.times().unwrap_err();
        assert!(
            error.contains("step_ms") && error.contains("Set"),
            "{error}"
        );
        r.selection = Selection::Uniform {
            start_ms: 0,
            end_ms: 4000,
            count: 2,
        };
        r.output = Output::Frame;
        assert!(r.times().unwrap_err().contains("Choose"));
        r.output = Output::ContactSheet;
        r.max_edge = Some(64);
        assert!(r.times().unwrap_err().contains("Increase"));
        r.max_edge = Some(0);
        assert!(r.times().unwrap_err().contains("Choose"));
        r.max_edge = None;
        r.path.clear();
        assert!(r.times().unwrap_err().contains("Provide"));
    }
    #[test]
    fn scene_sample_work_and_selected_frame_budgets_are_separate() {
        let mut r = request(Selection::SceneChanges {
            start_ms: 1000,
            end_ms: 61000,
            step_ms: 250,
            max_frames: 3,
        });
        assert_eq!(r.times().unwrap().len(), MAX_SCAN_SAMPLES);
        assert_eq!(r.frame_limit().unwrap(), 3);
        assert_eq!(r.preflight().unwrap(), "Up to 3 selected frames; 1 image");
        r.selection = Selection::SceneChanges {
            start_ms: 0,
            end_ms: 60250,
            step_ms: 250,
            max_frames: 24,
        };
        assert!(r.times().unwrap_err().contains("Increase step_ms"));
        for (start_ms, end_ms, step_ms, max_frames) in [
            (10, 10, 1, 24),
            (10, 9, 1, 24),
            (0, 1000, 0, 24),
            (0, 1000, 250, 0),
            (0, 1000, 250, 25),
        ] {
            r.selection = Selection::SceneChanges {
                start_ms,
                end_ms,
                step_ms,
                max_frames,
            };
            assert!(r.frame_limit().is_err());
        }
    }
    #[test]
    fn declared_ranges_and_timestamps_respect_duration() {
        for selection in [
            Selection::Uniform {
                start_ms: 0,
                end_ms: 4500,
                count: 1,
            },
            Selection::Interval {
                start_ms: 0,
                end_ms: 4500,
                every_ms: 5000,
                max_frames: 1,
            },
            Selection::SceneChanges {
                start_ms: 0,
                end_ms: 4500,
                step_ms: 5000,
                max_frames: 1,
            },
            Selection::Timestamps {
                times_ms: vec![4000],
            },
        ] {
            let error = request(selection)
                .validate_duration(4_000_000_000)
                .unwrap_err();
            assert!(error.contains("4000 ms"));
            assert!(error.contains("Choose"));
        }
        assert!(
            request(Selection::Uniform {
                start_ms: 1000,
                end_ms: 4000,
                count: 1
            })
            .validate_duration(4_000_000_000)
            .is_ok()
        );
    }
    #[test]
    fn tiny_sheet_refuses_before_opening_video() {
        let mut request = request(Selection::Uniform {
            start_ms: 0,
            end_ms: 4000,
            count: 24,
        });
        request.max_edge = Some(64);
        assert!(request.times().unwrap_err().contains("limit_exceeded"));
    }
    #[test]
    fn limits_reject_without_silently_resampling() {
        for selection in [
            Selection::Uniform {
                start_ms: 0,
                end_ms: 4000,
                count: 25,
            },
            Selection::Interval {
                start_ms: 0,
                end_ms: 4000,
                every_ms: 1,
                max_frames: 24,
            },
            Selection::Timestamps {
                times_ms: vec![0, 600_001],
            },
        ] {
            assert!(
                request(selection)
                    .times()
                    .unwrap_err()
                    .contains("limit_exceeded")
            );
        }
    }
}
