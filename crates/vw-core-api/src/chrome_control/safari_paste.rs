//! Native Paste checks the app clipboard owner before key dispatch; no clipboard bytes leave this guard.
//! Called only for an approved Full-access Safari paste; cancellation drops its asynchronous waits.
use crate::core::Shared;
use vw_platform::clipboard_content::{ClipboardContent, ClipboardFormat};
use zeroize::Zeroizing;
const REFUSAL: &str = "Paste clipboard content is unavailable or withheld by vault protection. Wait for the vault copy to expire or use clipboard.write with ordinary content before trying Paste again.";
fn ordinary(bytes: &[u8]) -> Result<(), String> {
    if vw_secrets::find(bytes).is_some() {
        Err("Paste would expose a registered secret. Copy ordinary content before trying Paste again.".into())
    } else {
        Ok(())
    }
}
pub(super) async fn check(shared: &Shared) -> Result<(), String> {
    let clipboard = shared.config.clipboard.as_ref().ok_or(REFUSAL)?;
    clipboard.check_paste().await.map_err(|_| REFUSAL)?;
    let mut available = false;
    if let Ok(text) = clipboard.read_text().await {
        let text = Zeroizing::new(text);
        ordinary(text.as_bytes())?;
        available = true;
    }
    // A page may prefer HTML or an image even when plain text is present. Inspect every exposed format.
    for format in [ClipboardFormat::Html, ClipboardFormat::Png] {
        if let Ok(content) = clipboard.read_content(format).await {
            let bytes = Zeroizing::new(match content {
                ClipboardContent::Html(value) => value.into_bytes(),
                ClipboardContent::Png(value) => value,
            });
            ordinary(&bytes)?;
            available = true;
        }
    }
    if available {
        clipboard
            .check_paste()
            .await
            .map_err(|_| REFUSAL.to_owned())
    } else {
        Err(REFUSAL.into())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };
    use vw_platform::clipboard::{ClipboardBackend, ClipboardError, ClipboardService};
    struct Fake {
        text: Arc<Mutex<String>>,
        html: Arc<Mutex<Option<String>>>,
    }
    impl ClipboardBackend for Fake {
        fn get_content(
            &mut self,
            format: ClipboardFormat,
        ) -> Result<ClipboardContent, ClipboardError> {
            if format == ClipboardFormat::Html {
                self.html
                    .lock()
                    .unwrap()
                    .clone()
                    .map(ClipboardContent::Html)
                    .ok_or(ClipboardError::Operation)
            } else {
                Err(ClipboardError::Operation)
            }
        }
        fn get_text(&mut self) -> Result<String, ClipboardError> {
            Ok(self.text.lock().unwrap().clone())
        }
        fn set_text(&mut self, text: &str) -> Result<(), ClipboardError> {
            *self.text.lock().unwrap() = text.into();
            Ok(())
        }
        fn clear(&mut self) -> Result<(), ClipboardError> {
            self.text.lock().unwrap().clear();
            Ok(())
        }
    }
    #[tokio::test]
    async fn native_paste_withholds_vault_copies_and_registered_values() {
        let html = Arc::new(Mutex::new(None));
        let external = Arc::new(Mutex::new("ordinary text".into()));
        let service = Arc::new(
            ClipboardService::with_backend(
                Box::new(Fake {
                    text: external.clone(),
                    html: html.clone(),
                }),
                Duration::from_secs(30),
            )
            .unwrap(),
        );
        let core = crate::offline_core::start_with("safari-paste-vault", |config, _| {
            config.clipboard = Some(service.clone());
        })
        .await;
        assert!(check(&core.shared).await.is_ok());
        service
            .copy(Zeroizing::new("live vault copy fixture".into()))
            .await
            .unwrap();
        assert!(
            check(&core.shared)
                .await
                .unwrap_err()
                .contains("vault protection")
        );
        *external.lock().unwrap() = "ordinary external replacement".into();
        assert!(
            check(&core.shared)
                .await
                .unwrap_err()
                .contains("vault protection"),
            "a live vault copy also withholds uninspectable rich representations"
        );
        let secret = "paste-registered-value-fixture-6fa2e804";
        vw_secrets::register(vw_secrets::SecretKind::VaultValue, secret);
        service.write_text(secret.into()).await.unwrap();
        assert!(
            check(&core.shared)
                .await
                .unwrap_err()
                .contains("registered secret")
        );
        service.write_text("ordinary again".into()).await.unwrap();
        assert!(check(&core.shared).await.is_ok());
        *html.lock().unwrap() = Some(format!("<b>{secret}</b>"));
        assert!(
            check(&core.shared)
                .await
                .unwrap_err()
                .contains("registered secret")
        );
        *html.lock().unwrap() = None;
        core.shutdown().await;
        service.shutdown().await;
    }
}
