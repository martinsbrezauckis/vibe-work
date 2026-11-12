//! Bounded HTML and PNG clipboard values shared by the serialized clipboard worker and native backend.
//! Called by explicit browser clipboard actions; no page script or clipboard monitor uses this module.
use super::{
    COPY_WAIT, ClipboardBackend, ClipboardError, ClipboardService, NativeClipboard, Request,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::oneshot;
/// Supported non-plain clipboard formats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipboardFormat {
    /// An HTML fragment.
    Html,
    /// A PNG image.
    Png,
}
/// Ordinary clipboard content, never a vault copy.
#[derive(Clone, PartialEq, Eq)]
pub enum ClipboardContent {
    /// UTF-8 HTML fragment.
    Html(String),
    /// Encoded PNG image.
    Png(Vec<u8>),
}
impl ClipboardContent {
    /// The representation carried by this value.
    #[must_use]
    pub fn format(&self) -> ClipboardFormat {
        match self {
            Self::Html(_) => ClipboardFormat::Html,
            Self::Png(_) => ClipboardFormat::Png,
        }
    }
    fn valid(&self) -> bool {
        match self {
            Self::Html(s) => s.len() <= 262_144,
            Self::Png(b) => b.len() <= 262_144 && image_dimensions(b).is_some(),
        }
    }
}
impl ClipboardService {
    /// Reads one ordinary rich clipboard format, withholding a pending vault copy.
    /// # Errors
    /// Unsupported format, unavailable clipboard or pending secret copy.
    pub async fn read_content(
        &self,
        format: ClipboardFormat,
    ) -> Result<ClipboardContent, ClipboardError> {
        let (reply, response) = oneshot::channel();
        self.sender
            .send(Request::ReadContent(format, reply))
            .map_err(|_| ClipboardError::Unavailable)?;
        tokio::time::timeout(COPY_WAIT, response)
            .await
            .map_err(|_| ClipboardError::TimedOut)?
            .map_err(|_| ClipboardError::Unavailable)?
    }
    /// Writes ordinary rich content and returns the actual content read back.
    /// # Errors
    /// Invalid content, unavailable clipboard or unconfirmed write outcome.
    pub async fn write_content(
        &self,
        value: ClipboardContent,
    ) -> Result<ClipboardContent, ClipboardError> {
        if !value.valid() {
            return Err(ClipboardError::Operation);
        }
        let (reply, response) = oneshot::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        self.sender
            .send(Request::WriteContent(value, Arc::clone(&cancelled), reply))
            .map_err(|_| ClipboardError::Unavailable)?;
        match tokio::time::timeout(COPY_WAIT, response).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(ClipboardError::Unavailable),
            Err(_) => {
                cancelled.store(true, Ordering::Release);
                Err(ClipboardError::TimedOut)
            }
        }
    }
}
pub(super) fn write(
    backend: &mut dyn ClipboardBackend,
    value: &ClipboardContent,
    cancelled: &AtomicBool,
    reply: oneshot::Sender<Result<ClipboardContent, ClipboardError>>,
) -> bool {
    if cancelled.load(Ordering::Acquire) || reply.is_closed() {
        return false;
    }
    if let Err(error) = backend.set_content(value) {
        let _ = reply.send(Err(error));
        return false;
    }
    let result = backend.get_content(value.format());
    let confirmed = result.as_ref().is_ok_and(|actual| same(actual, value));
    if cancelled.load(Ordering::Acquire) || reply.is_closed() {
        if confirmed {
            let _ = backend.clear();
        }
        return true;
    }
    if reply.send(result).is_err() && confirmed {
        let _ = backend.clear();
    }
    true
}
fn same(a: &ClipboardContent, b: &ClipboardContent) -> bool {
    if a == b {
        return true;
    }
    match (a, b) {
        (ClipboardContent::Png(a), ClipboardContent::Png(b)) => decode(a)
            .ok()
            .zip(decode(b).ok())
            .is_some_and(|(a, b)| a == b),
        _ => false,
    }
}
fn image_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || !bytes.starts_with(b"\x89PNG\r\n\x1a\n") || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    (width > 0
        && height > 0
        && width <= 8192
        && height <= 8192
        && u64::from(width) * u64::from(height) <= 16_777_216)
        .then_some((width, height))
}
fn decode(bytes: &[u8]) -> Result<image::RgbaImage, ClipboardError> {
    image_dimensions(bytes).ok_or(ClipboardError::Operation)?;
    image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
        .map(image::DynamicImage::into_rgba8)
        .map_err(|_| ClipboardError::Operation)
}
pub(super) fn native_write(
    backend: &mut NativeClipboard,
    value: &ClipboardContent,
) -> Result<(), ClipboardError> {
    match value {
        ClipboardContent::Html(html) => {
            super::html::available()?;
            backend
                .clipboard()?
                .set_html(html, None)
                .map_err(|_| ClipboardError::Operation)
        }
        ClipboardContent::Png(bytes) => {
            let image = decode(bytes)?;
            let (width, height) = image.dimensions();
            backend
                .clipboard()?
                .set_image(arboard::ImageData {
                    width: width as usize,
                    height: height as usize,
                    bytes: image.into_raw().into(),
                })
                .map_err(|_| ClipboardError::Operation)
        }
    }
}
pub(super) fn native_read(
    backend: &mut NativeClipboard,
    format: ClipboardFormat,
) -> Result<ClipboardContent, ClipboardError> {
    let value = match format {
        ClipboardFormat::Html => ClipboardContent::Html(super::html::read()?),
        ClipboardFormat::Png => {
            let raw = backend
                .clipboard()?
                .get_image()
                .map_err(|_| ClipboardError::Operation)?;
            if raw.width > 8192
                || raw.height > 8192
                || raw.width.saturating_mul(raw.height) > 16_777_216
            {
                return Err(ClipboardError::Operation);
            }
            let image = image::RgbaImage::from_raw(
                u32::try_from(raw.width).map_err(|_| ClipboardError::Operation)?,
                u32::try_from(raw.height).map_err(|_| ClipboardError::Operation)?,
                raw.bytes.into_owned(),
            )
            .ok_or(ClipboardError::Operation)?;
            let mut bytes = std::io::Cursor::new(Vec::new());
            image
                .write_to(&mut bytes, image::ImageFormat::Png)
                .map_err(|_| ClipboardError::Operation)?;
            ClipboardContent::Png(bytes.into_inner())
        }
    };
    if value.valid() {
        Ok(value)
    } else {
        Err(ClipboardError::Operation)
    }
}
