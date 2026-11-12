//! One immutable local document for a personal-browser preview, with no directory or disk-reading routes.
//! The core supplies bytes from its confined file loader; the page server applies the passive document policy.
use crate::csp::Policy;
use axum::http::StatusCode;

pub(super) struct Snapshot {
    pub path: String,
    mime: &'static str,
    bytes: Vec<u8>,
}
impl Snapshot {
    pub fn new(name: &str, bytes: Vec<u8>) -> std::io::Result<Self> {
        let ext = std::path::Path::new(name)
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let mime = match ext.as_str() {
            "html" | "htm" => "text/html; charset=utf-8",
            "pdf" => "application/pdf",
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "webp" => "image/webp",
            "avif" => "image/avif",
            "svg" => "image/svg+xml",
            "ico" => "image/x-icon",
            _ => {
                return Err(std::io::Error::other(
                    "Choose a PDF, HTML or image document.",
                ));
            }
        };
        if bytes.len() as u64 > crate::server::MAX_FILE {
            return Err(std::io::Error::other(
                "The local document exceeds 20 MiB; choose a smaller file.",
            ));
        }
        Ok(Self {
            path: format!("/document.{ext}"),
            mime,
            bytes,
        })
    }
    pub fn serve(&self, path: &str) -> Result<(&'static str, Vec<u8>, Policy), StatusCode> {
        if path != self.path {
            return Err(StatusCode::NOT_FOUND);
        }
        Ok((self.mime, self.bytes.clone(), Policy::Document))
    }
}
