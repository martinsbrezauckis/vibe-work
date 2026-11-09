//! Ordinary browser clipboard text shares the vault worker without exposing its pending secret copies.
//! Uses an in-memory backend to prove expiry and read behavior without a desktop.
#![allow(clippy::unwrap_used)]
use std::sync::{Arc, Mutex};
use std::time::Duration;
use vw_platform::clipboard::{ClipboardBackend, ClipboardError, ClipboardService};

struct Memory(Arc<Mutex<String>>);
impl ClipboardBackend for Memory {
    fn set_text(&mut self, value: &str) -> Result<(), ClipboardError> {
        *self.0.lock().unwrap() = value.into();
        Ok(())
    }
    fn get_text(&mut self) -> Result<String, ClipboardError> {
        Ok(self.0.lock().unwrap().clone())
    }
    fn clear(&mut self) -> Result<(), ClipboardError> {
        self.0.lock().unwrap().clear();
        Ok(())
    }
}

#[tokio::test]
async fn ordinary_text_round_trips_but_live_vault_copies_are_withheld() {
    let memory = Arc::new(Mutex::new(String::new()));
    let clipboard =
        ClipboardService::with_backend(Box::new(Memory(memory.clone())), Duration::from_secs(5))
            .unwrap();
    clipboard
        .write_text("fictional page work".into())
        .await
        .unwrap();
    assert_eq!(clipboard.read_text().await.unwrap(), "fictional page work");
    clipboard
        .copy(zeroize::Zeroizing::new("synthetic secret".into()))
        .await
        .unwrap();
    assert!(clipboard.read_text().await.is_err());
    *memory.lock().unwrap() = "replacement from another app".into();
    assert_eq!(
        clipboard.read_text().await.unwrap(),
        "replacement from another app"
    );
    clipboard.write_text(String::new()).await.unwrap();
    assert_eq!(clipboard.read_text().await.unwrap(), "");
    clipboard.shutdown().await;
}
