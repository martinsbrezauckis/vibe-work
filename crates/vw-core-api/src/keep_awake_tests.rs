//! Idle-sleep lifecycle falsifiers over a fake OS edge and real core state shapes.
//! Verifies one hold, same-thread release, refusal without retries, waits and subagents.

use crate::KeepAwakeStatus as Status;
use crate::keep_awake::{AwakeState, state_working};
use serde_json::json;
use std::sync::{Arc, Mutex};
use vw_platform::keep_awake::{IdleSleepBackend, IdleSleepHold, SleepRefused};

#[derive(Default)]
struct Fake {
    calls: Arc<Mutex<Vec<(&'static str, std::thread::ThreadId)>>>,
    refused: bool,
}
struct Hold(Arc<Mutex<Vec<(&'static str, std::thread::ThreadId)>>>);
impl IdleSleepHold for Hold {}
impl Drop for Hold {
    fn drop(&mut self) {
        self.0
            .lock()
            .unwrap()
            .push(("release", std::thread::current().id()));
    }
}
impl IdleSleepBackend for Fake {
    fn acquire(&self) -> Result<Box<dyn IdleSleepHold>, SleepRefused> {
        self.calls
            .lock()
            .unwrap()
            .push(("acquire", std::thread::current().id()));
        if self.refused {
            return Err(SleepRefused);
        }
        Ok(Box::new(Hold(self.calls.clone())))
    }
}

#[tokio::test]
async fn one_hold_for_two_chats_released_on_last_idle_and_on_setting_off() {
    let backend = Arc::new(Fake::default());
    let state = AwakeState::new(backend.clone(), true);
    state.working(true);
    state.working(true);
    state.enabled(true).await;
    assert_eq!(state.view().status, Status::Active);
    assert_eq!(backend.calls.lock().unwrap().len(), 1);
    state.working(true); // one chat done, another still running
    state.enabled(true).await;
    assert_eq!(backend.calls.lock().unwrap().len(), 1);
    state.working(false);
    state.enabled(true).await;
    assert_eq!(state.view().status, Status::Idle);
    state.working(true);
    state.enabled(false).await;
    assert_eq!(state.view().status, Status::Idle);
    assert_eq!(backend.calls.lock().unwrap().len(), 4);
    state.working(true);
    state.enabled(false).await;
    assert_eq!(backend.calls.lock().unwrap().len(), 4);
    state.enabled(true).await;
    assert_eq!(state.view().status, Status::Active);
    state.shutdown().await;
    let calls = backend.calls.lock().unwrap();
    assert_eq!(
        calls.iter().map(|call| call.0).collect::<Vec<_>>(),
        [
            "acquire", "release", "acquire", "release", "acquire", "release"
        ]
    );
    assert!(
        calls.iter().all(|call| call.1 == calls[0].1),
        "Windows requests are thread-bound"
    );
}

#[tokio::test]
async fn drop_releases_a_live_hold_before_returning() {
    let backend = Arc::new(Fake::default());
    let state = AwakeState::new(backend.clone(), true);
    state.working(true);
    state.enabled(true).await;
    drop(state);
    assert_eq!(backend.calls.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn refused_request_is_reported_once_without_a_retry_loop() {
    let backend = Arc::new(Fake {
        refused: true,
        ..Fake::default()
    });
    let state = AwakeState::new(backend.clone(), true);
    for _ in 0..10 {
        state.working(true);
    }
    state.enabled(true).await;
    assert_eq!(state.view().status, Status::Refused);
    assert_eq!(backend.calls.lock().unwrap().len(), 1);
    state.working(false);
    state.enabled(true).await;
    assert_eq!(state.view().status, Status::Idle);
    state.working(true);
    state.enabled(true).await;
    assert_eq!(backend.calls.lock().unwrap().len(), 2);
}

#[test]
fn running_turn_and_subagents_count_but_user_wait_done_stop_failure_do_not() {
    let mut state = json!({"session":{"status":{"type":"running"}},
        "turns":[{"status":"in_progress"}], "activeCallbacks":[], "childSessions":[]});
    assert!(state_working(&state, false));
    assert!(
        !state_working(&state, true),
        "gate callback, including a subagent's card"
    );
    state["activeCallbacks"] = json!([{"type":"user_input"}]);
    assert!(!state_working(&state, false));
    state["activeCallbacks"] = json!([]);
    state["session"]["status"] = json!({"type":"blocked"});
    assert!(!state_working(&state, false));
    for status in ["completed", "interrupted", "failed"] {
        state["turns"][0]["status"] = json!(status);
        state["session"]["status"] = json!({"type":"idle"});
        assert!(!state_working(&state, false), "{status}");
    }
    state["childSessions"] = json!([{"id":"child", "status":{"type":"running"}}]);
    assert!(state_working(&state, false));
    state["childSessions"][0]["status"] = json!({"type":"blocked"});
    assert!(!state_working(&state, false));
    state["turnQueue"] = json!({"items":[{}]});
    state["backgroundProcesses"] = json!([{"status":"running"}]);
    assert!(
        !state_working(&state, false),
        "queued work and left-running processes are not turns"
    );
}

#[test]
fn settings_default_on_persist_off_and_preserve_other_writers() {
    let dir = std::env::temp_dir().join(format!("vw-awake-settings-{}", std::process::id()));
    let path = dir.join("settings.json");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(crate::app_settings::keep_awake_enabled(&path));
    crate::write_app_settings(&path, json!({"theme":"dark"}).as_object().unwrap().clone()).unwrap();
    assert!(
        crate::app_settings::keep_awake_enabled(&path),
        "older settings default on"
    );
    crate::write_app_settings(
        &path,
        json!({"keepAwake":false}).as_object().unwrap().clone(),
    )
    .unwrap();
    assert!(!crate::app_settings::keep_awake_enabled(&path));
    crate::write_app_settings(&path, json!({"debugApi":true}).as_object().unwrap().clone())
        .unwrap();
    let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(
        saved,
        json!({"theme":"dark","keepAwake":false,"debugApi":true})
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn saved_off_is_applied_before_the_first_turn_and_published_with_status() {
    let backend = Arc::new(Fake::default());
    let state = AwakeState::new(backend.clone(), false);
    state.working(true);
    state.enabled(false).await;
    assert!(!state.view().enabled);
    assert_eq!(state.view().status, Status::Idle);
    assert!(backend.calls.lock().unwrap().is_empty());
    state.enabled(true).await;
    assert!(state.view().enabled);
    assert_eq!(state.view().status, Status::Active);
    state.enabled(false).await;
    assert!(!state.view().enabled);
    assert_eq!(state.view().status, Status::Idle);
    assert_eq!(backend.calls.lock().unwrap().len(), 2);
}

#[test]
fn another_subagent_still_working_keeps_the_hold_during_a_child_approval() {
    let state = json!({"session":{"status":{"type":"running"}},
        "turns":[{"status":"in_progress"}], "activeCallbacks":[],
        "childSessions":[{"id":"waiting", "status":{"type":"blocked"}},
                         {"id":"working", "status":{"type":"running"}}]});
    assert!(
        state_working(&state, true),
        "only all-waiting work releases the hold"
    );
}
