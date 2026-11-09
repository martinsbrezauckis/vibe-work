//! Marker failure logging contract: one warning per process, with the complete native error.
//! Uses a local tracing subscriber and local Once so parallel tests never consume the process's diagnostic.

use super::*;
use std::sync::Mutex;
use tracing::{Event, Metadata, Subscriber, field::Visit, span};

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<String>>>);

impl Visit for Capture {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0.lock().unwrap().push(format!("{field}={value:?}"));
    }
}

impl Subscriber for Capture {
    fn enabled(&self, _: &Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, _: &span::Attributes<'_>) -> span::Id {
        span::Id::from_u64(1)
    }
    fn record(&self, _: &span::Id, _: &span::Record<'_>) {}
    fn record_follows_from(&self, _: &span::Id, _: &span::Id) {}
    fn event(&self, event: &Event<'_>) {
        assert_eq!(*event.metadata().level(), tracing::Level::WARN);
        event.record(&mut self.clone());
    }
    fn enter(&self, _: &span::Id) {}
    fn exit(&self, _: &span::Id) {}
}

#[test]
fn click_marker_error_reports_the_failing_call_once_after_success() {
    let capture = Capture::default();
    let once = Once::new();
    let error = "CreateWindowExW failed (GetLastError=5, 0x00000005)";
    tracing::subscriber::with_default(capture.clone(), || {
        report_marker_result(&once, Ok(()));
        assert!(!once.is_completed());
        report_marker_result(&once, Err(crate::EngineError::Platform(error.into())));
        report_marker_result(
            &once,
            Err(crate::EngineError::Platform("second failure".into())),
        );
    });
    let fields = capture.0.lock().unwrap();
    assert_eq!(
        fields
            .iter()
            .filter(|field| field.starts_with("message="))
            .count(),
        1
    );
    assert!(fields.iter().any(|field| field.contains(error)));
    assert!(!fields.iter().any(|field| field.contains("second failure")));
}
