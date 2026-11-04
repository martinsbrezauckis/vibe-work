//! Native video decoder seam. Only the isolated helper calls it with an authorized private input.
//! OS adapters supply normalized presentation timestamps and bounded RGBA frames.
/// One decoded frame and its normalized presentation time.
#[derive(Debug)]
pub struct Frame {
    /// Presentation time in nanoseconds.
    pub pts_ns: u64,
    /// Pixel width.
    pub width: u32,
    /// Pixel height.
    pub height: u32,
    /// Packed RGBA8.
    pub pixels: Vec<u8>,
}
/// Source metadata; arbitrary file tags are never returned.
#[derive(Debug, Clone)]
pub struct Metadata {
    /// Visible duration.
    pub duration_ns: u64,
    /// Encoded width.
    pub width: u32,
    /// Encoded height.
    pub height: u32,
    /// Clockwise display rotation.
    pub rotation: u32,
    /// Source pixel aspect ratio.
    pub par: (u32, u32),
    /// Codec and profile from the selected video track.
    pub codec: String,
}
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::Decoder;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::Decoder;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::Decoder;
mod geometry;
#[cfg(target_os = "linux")]
pub mod limits;
#[cfg(test)]
mod tests;

mod container;
#[cfg(all(test, target_os = "linux"))]
mod plugin_tests;

/// Native backend identification; no user metadata or filesystem probes.
#[must_use]
pub fn backend() -> String {
    #[cfg(target_os = "linux")]
    {
        gstreamer::version_string().to_string()
    }
    #[cfg(windows)]
    {
        "Media Foundation (system; version unavailable)".into()
    }
    #[cfg(target_os = "macos")]
    {
        "AVFoundation (system; version unavailable)".into()
    }
}
