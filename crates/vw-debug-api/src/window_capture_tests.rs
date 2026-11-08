//! The shared feedback/Debug capture path must blank secret pixels, refuse unknown geometry and serialize captures.
use crate::{
    AppRequest, AppWindow, BoxFuture, UiRoutes, WindowRequest, WindowState, WindowView,
    capture_main,
};
use serde_json::json;
use std::sync::{Arc, Mutex};
struct RedWindow;
impl AppWindow for RedWindow {
    fn windows(&self) -> BoxFuture<'_, Result<Vec<WindowView>, String>> {
        Box::pin(async { Ok(Vec::new()) })
    }
    fn capture(&self, id: String) -> BoxFuture<'_, Result<Vec<u8>, String>> {
        assert_eq!(id, "main");
        Box::pin(async { vw_preview::encode(&vw_preview::canvas(40, 20, [255, 0, 0, 255])) })
    }
    fn control(&self, _: WindowRequest) -> BoxFuture<'_, Result<WindowState, String>> {
        Box::pin(async { Err("unused".into()) })
    }
    fn app_control(&self, _: AppRequest) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async { Err("unused".into()) })
    }
}
fn facts(x: i32) -> serde_json::Value {
    json!({"viewportWidth":40,"viewportHeight":20,"rects":[{"x":x,"y":5,"width":10,"height":5}],"epoch":1,"clear":0})
}
#[tokio::test]
async fn shared_capture_blanks_pixels_and_serializes_all_callers() {
    let routes = UiRoutes::default();
    let locks = Arc::new(Mutex::new(Vec::new()));
    let seen = locks.clone();
    routes
        .register("secrets", "synthetic geometry", move |body| {
            seen.lock().unwrap().push(body["lock"].as_bool().unwrap());
            async { Ok(facts(10)) }
        })
        .unwrap();
    let app: crate::SharedAppWindow = Arc::new(RedWindow);
    let other_routes = routes.clone();
    let (first, second) = tokio::join!(
        capture_main(&app, &routes),
        capture_main(&app, &other_routes)
    );
    for answer in [first, second] {
        let (png, count) = answer.unwrap();
        assert_eq!(count, 1);
        let image = vw_preview::decode(&png).unwrap();
        assert_eq!(image.pixel(10, 5), Some([128, 128, 128, 255]));
        assert_eq!(image.pixel(30, 15), Some([255, 0, 0, 255]));
    }
    assert_eq!(*locks.lock().unwrap(), [true, false, true, false]);
}
#[tokio::test]
async fn shared_capture_refuses_missing_or_moving_secret_fields() {
    let app: crate::SharedAppWindow = Arc::new(RedWindow);
    let routes = UiRoutes::default();
    assert!(
        capture_main(&app, &routes)
            .await
            .unwrap_err()
            .secret_check_failed()
    );
    routes
        .register("secrets", "moving geometry", |body| async move {
            Ok(facts(if body["lock"] == true { 10 } else { 20 }))
        })
        .unwrap();
    assert!(
        capture_main(&app, &routes)
            .await
            .unwrap_err()
            .secret_check_failed()
    );
}
#[tokio::test]
async fn malformed_secret_geometry_fails_closed() {
    let app: crate::SharedAppWindow = Arc::new(RedWindow);
    for changes in [
        json!({"width":null}),
        json!({"x":"unknown"}),
        json!({"height":-1}),
    ] {
        let routes = UiRoutes::default();
        routes
            .register("secrets", "invalid geometry", move |_| {
                let mut answer = facts(10);
                for (key, value) in changes.as_object().unwrap() {
                    answer["rects"][0][key] = value.clone();
                }
                async move { Ok(answer) }
            })
            .unwrap();
        assert!(
            capture_main(&app, &routes).await.is_err(),
            "Unknown secret geometry must never be treated as zero"
        );
    }
}
