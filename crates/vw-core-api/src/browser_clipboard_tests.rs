//! App clipboard readback uses the serialized backend; replacement content is observed and never called a match.
use super::*;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use vw_platform::clipboard::{ClipboardBackend, ClipboardError, ClipboardService};
struct Fake {
    value: Arc<Mutex<String>>,
    replacement: Option<String>,
}
impl ClipboardBackend for Fake {
    fn set_text(&mut self, text: &str) -> Result<(), ClipboardError> {
        *self.value.lock().unwrap() = self.replacement.clone().unwrap_or_else(|| text.into());
        Ok(())
    }
    fn get_text(&mut self) -> Result<String, ClipboardError> {
        Ok(self.value.lock().unwrap().clone())
    }
    fn clear(&mut self) -> Result<(), ClipboardError> {
        self.value.lock().unwrap().clear();
        Ok(())
    }
}
#[tokio::test]
async fn clipboard_text_write_and_clear_report_the_actual_readback() {
    for (name, replacement, wanted, expected, matched) in [
        (
            "changed",
            Some("changed by another owner".into()),
            "requested",
            "changed by another owner",
            false,
        ),
        ("clear", None, "", "", true),
    ] {
        let value = Arc::new(Mutex::new("old".into()));
        let service = Arc::new(
            ClipboardService::with_backend(
                Box::new(Fake { value, replacement }),
                Duration::from_secs(30),
            )
            .unwrap(),
        );
        let core =
            crate::offline_core::start_with(&format!("clipboard-readback-{name}"), |config, _| {
                config.clipboard = Some(service.clone());
            })
            .await;
        let result = run(
            &core.shared,
            &json!({"action":"clipboard.write","text":wanted}),
        )
        .await
        .unwrap();
        assert_eq!(result["readBack"], true);
        assert_eq!(result["matchesRequestedText"], matched);
        assert_eq!(result["untrustedPageContent"]["text"], expected);
        core.shutdown().await;
        service.shutdown().await;
    }
}
