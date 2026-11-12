//! Rich clipboard serialization and vault-copy exclusion with a deterministic in-memory backend.
#![allow(clippy::unwrap_used)]
use std::sync::{Arc, Mutex};
use std::time::Duration;
use vw_platform::clipboard::{ClipboardBackend, ClipboardError, ClipboardService};
use vw_platform::clipboard_content::{ClipboardContent, ClipboardFormat};
struct Memory {
    text: String,
    content: Arc<Mutex<Option<ClipboardContent>>>,
}
impl ClipboardBackend for Memory {
    fn set_text(&mut self, text: &str) -> Result<(), ClipboardError> {
        self.text = text.into();
        Ok(())
    }
    fn get_text(&mut self) -> Result<String, ClipboardError> {
        Ok(self.text.clone())
    }
    fn clear(&mut self) -> Result<(), ClipboardError> {
        self.text.clear();
        *self.content.lock().unwrap() = None;
        Ok(())
    }
    fn get_content(&mut self, _: ClipboardFormat) -> Result<ClipboardContent, ClipboardError> {
        self.content
            .lock()
            .unwrap()
            .clone()
            .ok_or(ClipboardError::Operation)
    }
    fn set_content(&mut self, c: &ClipboardContent) -> Result<(), ClipboardError> {
        self.text.clear();
        *self.content.lock().unwrap() = Some(c.clone());
        Ok(())
    }
}
#[tokio::test]
async fn rich_content_round_trips_and_a_live_vault_copy_is_withheld() {
    let content = Arc::new(Mutex::new(None));
    let service = ClipboardService::with_backend(
        Box::new(Memory {
            text: String::new(),
            content: content.clone(),
        }),
        Duration::from_secs(5),
    )
    .unwrap();
    let wanted = ClipboardContent::Html("<b>ordinary</b>".into());
    assert!(service.write_content(wanted.clone()).await.is_ok());
    assert!(service.read_content(ClipboardFormat::Html).await.unwrap() == wanted);
    service
        .copy(zeroize::Zeroizing::new("synthetic secret".into()))
        .await
        .unwrap();
    assert!(service.read_content(ClipboardFormat::Html).await.is_err());
    assert!(service.read_content(ClipboardFormat::Png).await.is_err());
    service.write_content(wanted.clone()).await.unwrap();
    assert!(service.read_content(ClipboardFormat::Html).await.unwrap() == wanted);
    service.shutdown().await;
}
