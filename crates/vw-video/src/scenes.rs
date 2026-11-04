//! Small deterministic scene comparisons inside the existing worker. The parent checks scan accounting;
//! samples are transient and use the same decoder, deadline and limits as selected frames.
#[cfg(feature = "helper")]
use crate::protocol::Frame;
use serde::{Deserialize, Serialize};
use vw_agent_tools::video::{Request, Selection};

/// Completed fixed-step scan, including work that produced no returned image.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scan {
    /// Inclusive baseline request time.
    pub start_ms: u64,
    /// Exclusive requested end (not a claim of continuous observation).
    pub end_ms: u64,
    /// Distance between comparison requests.
    pub step_ms: u64,
    /// Number of comparison samples, including the baseline.
    pub samples: usize,
    /// Last sampled request; the tail before `end_ms` was not sampled.
    pub last_sample_ms: u64,
    /// All changes detected across the range, even after the return cap filled.
    pub detected_changes: usize,
    /// Detected changes omitted from the returned frames by the cap.
    pub omitted_changes: usize,
}
impl Scan {
    pub(crate) fn planned(request: &Request) -> Result<Option<Self>, String> {
        let Selection::SceneChanges {
            start_ms,
            end_ms,
            step_ms,
            ..
        } = request.selection
        else {
            return Ok(None);
        };
        let times = request.times()?;
        Ok(Some(Self {
            start_ms,
            end_ms,
            step_ms,
            samples: times.len(),
            last_sample_ms: times.last().copied().unwrap_or(start_ms),
            detected_changes: 0,
            omitted_changes: 0,
        }))
    }
    pub(crate) fn validate(&self, request: &Request) -> Result<usize, String> {
        let invalid = || "decode_failed: invalid scene scan accounting".to_owned();
        let mut expected = Self::planned(request)?.ok_or_else(invalid)?;
        let cap = request.frame_limit()?;
        if self.detected_changes >= self.samples {
            return Err(invalid());
        }
        expected.detected_changes = self.detected_changes;
        expected.omitted_changes = self.detected_changes.saturating_sub(cap);
        if *self != expected {
            return Err(invalid());
        }
        Ok(self.detected_changes.min(cap))
    }
    pub(crate) fn coverage(&self) -> String {
        format!(
            "Sampled {}..{} ms (exclusive end) every {} ms: {} samples, last at {} ms; {} changes detected, {} omitted by the frame cap. The first sample is a baseline. Cuts are localized only between consecutive samples; subtle changes, intervening events and the unsampled tail may be missed. Request a narrower range, smaller step_ms or explicit timestamps to inspect further; do not infer continuous coverage.",
            self.start_ms,
            self.end_ms,
            self.step_ms,
            self.samples,
            self.last_sample_ms,
            self.detected_changes,
            self.omitted_changes
        )
    }
}

#[cfg(feature = "helper")]
pub(crate) fn select(
    decoder: &mut vw_platform::video::Decoder,
    request: &Request,
) -> Result<(Vec<Frame>, Scan), String> {
    let mut scan = Scan::planned(request)?.ok_or("decode_failed: missing scene selection")?;
    let cap = request.frame_limit()?;
    let mut previous: Option<vw_platform::video::Frame> = None;
    let mut selected = Vec::new();
    for ms in request.times()? {
        let frame = decoder.frame(ms * 1_000_000, 64)?;
        if let Some(before) = &previous {
            if before.width != frame.width
                || before.height != frame.height
                || before.pixels.len() != frame.pixels.len()
            {
                return Err(
                    "decode_failed: changing scene geometry. Request explicit timestamps instead."
                        .into(),
                );
            }
            // Integer RGB mean absolute difference, alpha excluded. A fixed threshold avoids model,
            // floating-point or host-tuning dependence. This is a cut heuristic, not semantic analysis.
            let difference: u64 = before
                .pixels
                .as_chunks::<4>()
                .0
                .iter()
                .zip(frame.pixels.as_chunks::<4>().0.iter())
                .map(|(a, b)| (0..3).map(|c| u64::from(a[c].abs_diff(b[c]))).sum::<u64>())
                .sum();
            let channels = u64::from(frame.width) * u64::from(frame.height) * 3;
            if frame.pts_ns != before.pts_ns && difference >= channels * 30 {
                scan.detected_changes += 1;
                if selected.len() < cap {
                    selected.push(Frame {
                        index: selected.len(),
                        requested_ms: ms,
                        actual_ns: frame.pts_ns,
                        frame_id: selected.len(),
                    });
                }
            }
        }
        previous = Some(frame);
    }
    scan.omitted_changes = scan.detected_changes.saturating_sub(cap);
    Ok((selected, scan))
}
