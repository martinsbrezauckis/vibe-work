//! Action normalization and clock contracts. Runs without a native window; no action content is needed.
use super::*;

#[test]
fn built_in_points_and_external_target_boxes_share_viewport_coordinates() {
    let native = ActionGeometry {
        point: Some(ActionPoint { x: 50.0, y: 75.0 }),
        viewport: Some((200.0, 100.0)),
        ..Default::default()
    };
    let external = ActionGeometry {
        target_box: Some(ActionBox {
            x: 30.0,
            y: 65.0,
            width: 40.0,
            height: 20.0,
        }),
        viewport: native.viewport,
        ..Default::default()
    };
    assert_eq!(native.normalized_point(), Some((0.25, 0.75)));
    assert_eq!(external.normalized_point(), native.normalized_point());
}

#[test]
fn clipped_target_uses_visible_center_but_invalid_points_are_never_clamped() {
    let mut geometry = ActionGeometry {
        target_box: Some(ActionBox {
            x: -40.0,
            y: 10.0,
            width: 80.0,
            height: 20.0,
        }),
        viewport: Some((200.0, 100.0)),
        ..Default::default()
    };
    assert_eq!(geometry.normalized_point(), Some((0.1, 0.2)));
    for x in [f64::NAN, f64::INFINITY, -1.0, 200.0] {
        geometry.point = Some(ActionPoint { x, y: 20.0 });
        assert_eq!(geometry.normalized_point(), None);
    }
    geometry.point = None;
    for viewport in [None, Some((0.0, 100.0)), Some((f64::INFINITY, 100.0))] {
        geometry.viewport = viewport;
        assert_eq!(geometry.normalized_point(), None);
    }
}

#[test]
fn a_tracker_assigns_order_and_time_without_retaining_history_for_new_views() {
    let tracker = ActionTracker::default();
    let mut stream = tracker.subscribe();
    let event = tracker.record(
        "chat".into(),
        ActionKind::Type,
        ActionSource {
            browser: SourceBrowser::Edge,
            view_id: Some("3".into()),
            document_id: None,
            profile_id: Some("edge:profile".into()),
            browser_session_id: Some("lease".into()),
        },
        ActionGeometry::default(),
    );
    assert_eq!(stream.try_recv().unwrap(), event);
    assert!(tracker.subscribe().try_recv().is_err());
    assert_eq!(event.sequence, 1);
    assert!(tracker.remaining(&event).is_some());
    let encoded = serde_json::to_string(&event).unwrap();
    assert!(!encoded.contains("text"));
    let mut future = event;
    future.at_ms = u64::MAX;
    assert!(tracker.remaining(&future).is_none());
}

#[test]
fn overlay_rejects_wrong_browser_profile_lease_tab_document_session_and_old_events() {
    let tracker = ActionTracker::default();
    let source = ActionSource {
        browser: SourceBrowser::Chrome,
        view_id: Some("7".into()),
        document_id: Some("document".into()),
        profile_id: Some("chrome:one".into()),
        browser_session_id: Some("lease".into()),
    };
    let event = tracker.record(
        "chat".into(),
        ActionKind::Click,
        source.clone(),
        ActionGeometry {
            point: Some(ActionPoint { x: 40.0, y: 20.0 }),
            viewport: Some((200.0, 100.0)),
            ..Default::default()
        },
    );
    assert_eq!(
        tracker.marker_for(&event, "chat", &source).unwrap().point,
        (0.2, 0.2)
    );
    assert!(
        tracker
            .marker_for(&event, "another-chat", &source)
            .is_none()
    );
    for changed in [
        ActionSource {
            browser: SourceBrowser::Edge,
            ..source.clone()
        },
        ActionSource {
            view_id: Some("8".into()),
            ..source.clone()
        },
        ActionSource {
            document_id: Some("next-document".into()),
            ..source.clone()
        },
        ActionSource {
            profile_id: Some("chrome:two".into()),
            ..source.clone()
        },
        ActionSource {
            browser_session_id: Some("next-lease".into()),
            ..source.clone()
        },
    ] {
        assert!(tracker.marker_for(&event, "chat", &changed).is_none());
    }
    let expired = ActionTracker {
        start: Instant::now().checked_sub(Duration::from_secs(1)).unwrap(),
        ..Default::default()
    };
    assert!(expired.marker_for(&event, "chat", &source).is_none());
}
