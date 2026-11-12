//! Final video wording, selected solely by typed outcomes. Called by every agent-facing video boundary.
use crate::{Code, Media};
/// Reason and instruction for cached calls when video is unavailable.
pub const UNAVAILABLE: &str = if cfg!(target_os = "linux") {
    "Not run: video review is not available on Linux yet. Ask the user for still images."
} else {
    "Not run: video review is unavailable in this app. Ask the user for still images."
};
const MEDIA_ACTION: &str = "Ask the user for still images or a copy in a supported format, or continue with metadata already available. Do not claim to have seen frames that were not returned.";
const WORKER_ACTION: &str = "Ask the user for still images and report this failure. This does not establish whether the video format is supported; do not retry the same extraction.";
#[allow(
    clippy::too_many_lines,
    reason = "one exhaustive mapping keeps each typed outcome and recovery instruction together"
)]
pub(super) fn outcome(code: &Code) -> (String, &'static str) {
    let (row, action) = match code {
        Code::Unavailable => (
            "Video review is unavailable in this app.",
            "Ask the user for still images.",
        ),
        Code::InvalidRequest => (
            "The video request is invalid.",
            "Supply integer millisecond times, a supported selection and an output mode.",
        ),
        Code::InvalidPath => (
            "The video path is invalid.",
            "Provide a nonempty local video path without NUL bytes, at most 4096 bytes long.",
        ),
        Code::Selection => (
            "The selected video range or frame count exceeds the request limits.",
            "Select 1–24 frames over a positive range of at most ten minutes within the first two hours.",
        ),
        Code::Step => (
            "The scene sampling step must be positive.",
            "Set step_ms to at least 1 millisecond.",
        ),
        Code::ScanSamples => (
            "Scene scanning allows at most 240 samples.",
            "Increase step_ms or choose a shorter range.",
        ),
        Code::FrameOutput => (
            "Frame output requires a one-frame selection or scene cap.",
            "Choose contact_sheet output or set the frame count to one.",
        ),
        Code::ImageEdge => (
            "The output image size cannot hold the selected frames.",
            "Use max_edge of at least 64; increase it or reduce the frame count for a contact sheet.",
        ),
        Code::OutsideDuration { duration_ns } => {
            return (
                format!(
                    "The selection exceeds the video duration of {duration_ns} ns ({} ms).",
                    duration_ns / 1_000_000
                ),
                "Choose start_ms before the duration and end_ms no later than it; explicit timestamps must be before the duration.",
            );
        }
        Code::InputSize => (
            "The video file is outside the input limits.",
            "Ask the user for a nonempty, smaller video file of at most 512 MiB.",
        ),
        Code::Duration => (
            "The video duration is outside the supported range.",
            "Ask the user for a nonempty video no longer than two hours.",
        ),
        Code::Dimensions => (
            "The video's dimensions are outside the decoding limits.",
            "Ask the user for a smaller video, at most 4096 pixels per side and 8,847,360 pixels per frame, or still images.",
        ),
        Code::PixelAspect => (
            "The video's pixel aspect ratio is unsupported.",
            "Ask the user for a copy with square pixels or still images.",
        ),
        Code::Rotation => (
            "This video's rotation or reflection is unsupported.",
            "Ask the user for a copy with the display rotation applied to its pixels, or still images.",
        ),
        Code::Hdr => (
            "HDR color conversion is unavailable for this video.",
            "Ask the user for an SDR copy or still images.",
        ),
        Code::MetadataLimit => (
            "The video's container metadata exceeds the reading limits.",
            "Ask the user for a simpler or shorter exported copy, or still images.",
        ),
        Code::DecodeLimit => (
            "Reading this video exceeded the sample or timeline limits.",
            "Choose a shorter range or ask the user for a shorter copy or still images.",
        ),
        Code::OutputLimit => (
            "The extracted video result exceeds the output size limit.",
            "Reduce max_edge or the selected frame count, then request the selection again.",
        ),
        Code::OutputEncoding => (
            "The selected video frames could not be encoded as an image.",
            "Reduce max_edge or the selected frame count, or ask the user for still images.",
        ),
        Code::Memory => (
            "Video extraction exceeded its memory limit.",
            "Ask the user for a smaller video with lower resolution or still images.",
        ),
        Code::Timeout => (
            "Video extraction timed out.",
            "Choose a shorter range or fewer samples, or ask the user for still images.",
        ),
        Code::ChangedInput => (
            "The video file changed or could not be read consistently.",
            "Use a readable local copy after it has finished changing, then request frames from that file.",
        ),
        Code::Cancelled => (
            "Video extraction ended before completion.",
            "Continue only after the user's next message if they still want video frames.",
        ),
        Code::ModelImages => (
            "This model has no confirmed image support.",
            "Use metadata output, or ask the user to choose an image-capable model for frames.",
        ),
        Code::Busy => (
            "Video extraction is busy.",
            "Wait for an active video job to finish before requesting another.",
        ),
        Code::ImageAllowance => (
            "This message's video image allowance is used.",
            "Use the images already returned; more can be taken after the user's next message. Do not retry in this turn.",
        ),
        Code::NoImageTurn => (
            "There is no current user turn for these video images.",
            "Wait for the user's next message before requesting frames.",
        ),
        Code::NotFound { path } => {
            return (
                format!("Not found: {path}."),
                "Check the file name and folder.",
            );
        }
        Code::CannotRead { path } => {
            return (
                format!("Cannot read: {path}."),
                "Check that this path names a readable local file.",
            );
        }
        Code::ReadDenied => (
            "This video's location is excluded by the file access rules.",
            "Choose a permitted local video file; ask the user to provide one if needed.",
        ),
        Code::FolderRefused { reason } => {
            return (
                reason.clone(),
                "Use a permitted video file after resolving this folder refusal.",
            );
        }
        Code::FolderCheck => (
            "Vibe Work could not establish access to the video's folder.",
            "Ask the user to check that the chat folder and its links are accessible, then reopen this chat.",
        ),
        Code::InputLinks => (
            "This video's path contains a symbolic link, which video reading does not support.",
            "Use a regular local copy in an accessible folder.",
        ),
        Code::InputStaging => (
            "Vibe Work could not prepare a stable local copy of the video.",
            "Ask the user to check available temporary storage and retry, or provide still images.",
        ),
        Code::WorkerMissing => (
            "The video component is missing from the installation.",
            "Ask the user to repair the Vibe Work installation or provide still images.",
        ),
        Code::WorkerStart => ("Video extraction could not start.", WORKER_ACTION),
        Code::WorkerIsolation => (
            "The required protection for video extraction could not be established.",
            WORKER_ACTION,
        ),
        Code::WorkerFailed => ("The video process stopped unexpectedly.", WORKER_ACTION),
        Code::InvalidReply => (
            "Video extraction returned an invalid or incomplete result.",
            WORKER_ACTION,
        ),
        Code::MediaUnavailable { media } => {
            return (
                format!(
                    "Cannot decode this video: {} media support is missing.",
                    match media {
                        Media::Mp4 => "MP4",
                        Media::Webm => "WebM",
                        Media::Gstreamer => "GStreamer",
                        Media::AvFoundation => "AVFoundation",
                        Media::MediaFoundation => "Media Foundation",
                    }
                ),
                MEDIA_ACTION,
            );
        }
        Code::MediaInitialization { .. } => (
            "The installed media support could not initialize for video extraction.",
            "Ask the user to reopen Vibe Work and try again, or provide still images.",
        ),
        Code::UnsupportedContainer => ("This video container format is unsupported.", MEDIA_ACTION),
        Code::ExternalMedia => (
            "This video refers to media stored in other files, which video reading does not support.",
            "Ask the user for a self-contained video file or still images.",
        ),
        Code::UnsupportedCodec { codec: Some(codec) } => {
            return (
                format!(
                    "Cannot decode this video: {} support is missing or unsupported.",
                    codec.label()
                ),
                MEDIA_ACTION,
            );
        }
        Code::UnsupportedCodec { codec: None } => (
            "Cannot decode this video: its codec is unsupported on this computer.",
            MEDIA_ACTION,
        ),
        Code::MediaOpen => (
            "The video could not be opened for extraction.",
            WORKER_ACTION,
        ),
        Code::NoVideoTrack => (
            "No usable video track was found in this file.",
            "Ask the user for a file containing a video track or still images.",
        ),
        Code::DisplayConversion => (
            "This video's pixel format cannot be converted to display colors.",
            "Ask the user for an SDR copy with a supported pixel format or still images.",
        ),
        Code::DecodeFailed => (
            "The installed video decoder could not read this video.",
            "Ask the user for a freshly exported copy or still images. This failure alone does not show that codec support is missing.",
        ),
        Code::FrameMissing => (
            "No complete video frame was available at a requested time.",
            "Choose another timestamp or ask the user for a freshly exported copy or still images.",
        ),
        Code::SceneGeometry => (
            "The video changes frame dimensions during the selected scene scan.",
            "Request explicit timestamps instead.",
        ),
        Code::HistorySave => (
            "Vibe Work could not save the extracted result for this chat.",
            "Ask the user to check the chat folder's available storage and access, then request frames in a new message.",
        ),
    };
    (row.into(), action)
}
