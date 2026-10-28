//! Private staging for native PDF writers. Called by the three webview backends; no native writer sees a
//! capture destination. The callback owns staging until printing ends, even if the caller cancels its future.
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

/// Private native output, removed on completion or error.
pub struct PdfFile {
    dir: PathBuf,
}
impl PdfFile {
    /// Creates a fresh user-only staging directory.
    ///
    /// # Errors
    /// The staging directory could not be created privately.
    pub fn new() -> Result<Self, String> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "vw-print-{}-{time}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
        let file = Self { dir };
        crate::private_file::ensure_private_dir(&file.dir).map_err(|e| e.to_string())?;
        Ok(file)
    }
    /// The native writer's output path, never a page-chosen path.
    #[must_use]
    pub fn path(&self) -> PathBuf {
        self.dir.join("page.pdf")
    }
    /// Reads the finished PDF with a size bound.
    ///
    /// # Errors
    /// Printing failed, the file is oversized, or it is not a PDF.
    pub fn read(&self) -> Result<Vec<u8>, String> {
        let path = self.path();
        let meta = std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if !meta.is_file() || meta.len() > 50 * 1024 * 1024 {
            return Err("the PDF exceeds 50 MB".into());
        }
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        if !bytes.starts_with(b"%PDF-") {
            return Err("the engine did not produce a PDF".into());
        }
        Ok(bytes)
    }
}
impl Drop for PdfFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
