//! Video errors shared across native adapters, worker transport and final agent text, without OS dependencies.
//! Producers choose codes at the failed operation; diagnostic strings are logs, never classifiers or agent text.
use serde::{Deserialize, Serialize};
mod text;
pub use text::UNAVAILABLE;

/// The known video coding formats; unknown native names remain diagnostic details.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Codec {
    /// AVC.
    H264,
    /// H.265.
    Hevc,
    /// `AOMedia` Video 1.
    Av1,
    /// VP8.
    Vp8,
    /// VP9.
    Vp9,
}
impl Codec {
    /// Human-readable media name.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::H264 => "H.264",
            Self::Hevc => "HEVC",
            Self::Av1 => "AV1",
            Self::Vp8 => "VP8",
            Self::Vp9 => "VP9",
        }
    }
}
/// Installed media support that an operation proved missing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Media {
    /// MP4 demuxing.
    Mp4,
    /// `WebM` demuxing.
    Webm,
    /// `GStreamer` runtime or common elements.
    Gstreamer,
    /// `AVFoundation` initialization.
    AvFoundation,
    /// Media Foundation initialization.
    MediaFoundation,
}
/// An outcome selected by the operation that failed, independent of its diagnostic wording.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "snake_case", deny_unknown_fields)]
pub enum Code {
    /// The tool is unavailable in this app/platform.
    Unavailable,
    /// The request does not match the tool schema.
    InvalidRequest,
    /// The model provided an invalid local path.
    InvalidPath,
    /// Frame count, timestamps or range exceed request limits.
    Selection,
    /// Scene sampling requires a positive step.
    Step,
    /// Scene scanning exceeds 240 comparisons.
    ScanSamples,
    /// Single-frame output was requested with multiple frames.
    FrameOutput,
    /// The requested output edge cannot hold the selected frames.
    ImageEdge,
    /// The declared range extends past the visible duration.
    OutsideDuration {
        /// Exact available duration.
        duration_ns: u64,
    },
    /// Empty, nonregular or oversized source.
    InputSize,
    /// Empty or over-two-hour duration.
    Duration,
    /// Source geometry exceeds native decoding limits.
    Dimensions,
    /// Unsupported pixel aspect ratio.
    PixelAspect,
    /// Unsupported display rotation or reflection.
    Rotation,
    /// HDR conversion is unsupported.
    Hdr,
    /// Container structure exceeds bounded metadata limits.
    MetadataLimit,
    /// Decoding exceeds bounded sample or timestamp limits.
    DecodeLimit,
    /// Encoded result exceeds the output limit.
    OutputLimit,
    /// The selected frames could not be encoded as an image.
    OutputEncoding,
    /// The worker exceeded its memory allowance.
    Memory,
    /// Wall-clock or CPU deadline.
    Timeout,
    /// Input changed or could not be read consistently.
    ChangedInput,
    /// Turn, credential or job ended before delivery.
    Cancelled,
    /// Model image support is not confirmed.
    ModelImages,
    /// Job concurrency is exhausted.
    Busy,
    /// The turn has used its image allowance.
    ImageAllowance,
    /// No current user turn can own the images.
    NoImageTurn,
    /// The supplied file does not exist.
    NotFound {
        /// User-supplied source path.
        path: String,
    },
    /// The supplied path is not a readable local file.
    CannotRead {
        /// User-supplied source path.
        path: String,
    },
    /// The file access policy refuses the source.
    ReadDenied,
    /// Folder access could not be established.
    FolderCheck,
    /// An already-presented folder refusal, produced only by the core's access policy.
    FolderRefused {
        /// The shared folder policy's diagnosis and instruction.
        reason: String,
    },
    /// Source uses unsupported symbolic links.
    InputLinks,
    /// A private stable source copy could not be prepared.
    InputStaging,
    /// The installed video program is absent.
    WorkerMissing,
    /// The video process could not be started.
    WorkerStart,
    /// Required isolation could not be established.
    WorkerIsolation,
    /// The running video process failed.
    WorkerFailed,
    /// The worker did not return a valid complete result.
    InvalidReply,
    /// Native runtime or container support is absent.
    MediaUnavailable {
        /// Support checked at the failing operation.
        media: Media,
    },
    /// Installed native media support could not initialize.
    MediaInitialization {
        /// The media system that failed to initialize.
        media: Media,
    },
    /// The container is unsupported.
    UnsupportedContainer,
    /// The container refers to external media.
    ExternalMedia,
    /// Native codec support is missing or unsupported.
    UnsupportedCodec {
        /// Known codec, if identified by native metadata.
        codec: Option<Codec>,
    },
    /// The installed decoder could not read the source.
    DecodeFailed,
    /// The media API could not open the authorized source.
    MediaOpen,
    /// No usable video track was found.
    NoVideoTrack,
    /// Native pixels could not be converted to display colors.
    DisplayConversion,
    /// No complete covering frame was returned.
    FrameMissing,
    /// Scene sampling encountered changing frame geometry.
    SceneGeometry,
    /// The extracted result could not be saved for this chat.
    HistorySave,
}
/// Typed error transported unchanged from producer to the final presentation boundary.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Error {
    /// Machine-readable outcome; never inferred from `detail`.
    pub code: Code,
    /// Internal diagnosis for logs. Never included in an agent response.
    pub detail: String,
}
impl Error {
    /// Construct at the operation that failed, retaining its diagnostic detail.
    #[must_use]
    #[allow(
        clippy::needless_pass_by_value,
        reason = "map_err consumes native errors; only their bounded display text is retained"
    )]
    pub fn new(code: Code, detail: impl ToString) -> Self {
        Self {
            code,
            detail: detail.to_string().chars().take(2048).collect(),
        }
    }
    /// Retain the failed operation and native cause together for logging.
    #[must_use]
    #[allow(
        clippy::needless_pass_by_value,
        reason = "map_err consumes native errors; only their bounded diagnostic text is retained"
    )]
    pub fn at(code: Code, operation: &str, cause: impl std::fmt::Debug) -> Self {
        Self::new(code, format!("{operation}: {cause:?}"))
    }
    /// One diagnosis and instruction for the agent.
    #[must_use]
    pub fn agent(&self) -> String {
        if self.code == Code::Unavailable {
            return UNAVAILABLE.into();
        }
        if let Code::FolderRefused { reason } = &self.code {
            return format!("{reason} No video images were sent.");
        }
        let (row, instruction) = text::outcome(&self.code);
        format!("{row} No video images were sent. {instruction}")
    }
    /// Short diagnosis for the existing result row.
    #[must_use]
    pub fn row(&self) -> String {
        text::outcome(&self.code).0
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.code, self.detail)
    }
}
impl std::error::Error for Error {}
