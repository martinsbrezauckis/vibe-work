//! Regression tests for `process_lifecycle.rs`; uses the parent module and is called by Cargo tests.
use super::{FinishedHere, forget_finished, handoff_finished, remember_finished};
use std::collections::VecDeque;
use vw_procmon::{ProcessRef, StartToken};

#[tokio::test]
async fn an_unowned_live_process_is_refused_without_claiming_it_is_dead() {
    let fixture = crate::feature_test_core::Fixture::new().await;
    let mut command = if cfg!(windows) {
        let mut command = std::process::Command::new("ping");
        command.args(["-n", "30", "127.0.0.1"]);
        command
    } else {
        let mut command = std::process::Command::new("sleep");
        command.arg("30");
        command
    };
    let mut child = command.stdout(std::process::Stdio::null()).spawn().unwrap();
    let shared = &fixture.core.shared;
    let result = super::process_stop(
        shared,
        shared.agent_tools.as_ref().unwrap(),
        "unowned",
        &serde_json::json!({"pid":child.id()}),
    );
    let still_running = child.try_wait().unwrap().is_none();
    let _ = child.kill();
    child.wait().unwrap();
    fixture.finish().await;
    let reason = result.unwrap_err();
    eprintln!("unowned live pid refusal: {reason}");
    assert!(still_running, "an unowned child must not be stopped");
    assert!(!reason.contains("not running"), "{reason}");
    assert!(reason.contains("not stopped"), "{reason}");
}

#[test]
fn a_stale_scan_does_not_finish_a_new_live_launch() {
    let process = vw_procmon::identity(std::process::id()).unwrap();
    let stale = vw_procmon::Snapshot {
        listeners: Vec::new(),
        processes: std::collections::BTreeMap::new(),
        me: vw_procmon::SelfInfo {
            pid: process.pid,
            pgid: None,
        },
        took: std::time::Duration::ZERO,
    };
    assert!(!super::ended(&stale, process));
}

fn entry(session: &str, pid: u32) -> FinishedHere {
    FinishedHere {
        session_id: session.to_owned(),
        process: ProcessRef {
            pid,
            start: StartToken(u64::from(pid)),
        },
        status: "stopped".to_owned(),
        command: "server".to_owned(),
        shell: "test".to_owned(),
        last_output: format!("output-{pid}"),
    }
}

#[test]
fn finished_output_keeps_newest_eight_per_session_and_separates_sessions() {
    let mut rows = VecDeque::new();
    remember_finished(&mut rows, entry("other", 99));
    for pid in 1..=10 {
        remember_finished(&mut rows, entry("mine", pid));
    }
    assert_eq!(rows.len(), 9);
    assert_eq!(rows.front().unwrap().process.pid, 99);
    assert_eq!(
        rows.iter()
            .filter(|s| s.session_id == "mine")
            .map(|s| s.process.pid)
            .collect::<Vec<_>>(),
        (3..=10).collect::<Vec<_>>()
    );
    remember_finished(&mut rows, entry("mine", 7));
    assert_eq!(rows.len(), 9);
    assert_eq!(rows.back().unwrap().last_output, "output-10");
    let mut repeated = entry("mine", 7);
    repeated.last_output.clear();
    remember_finished(&mut rows, repeated);
    assert_eq!(
        rows.iter()
            .find(|s| s.process.pid == 7)
            .unwrap()
            .last_output,
        "output-7"
    );
}

#[test]
fn finished_output_has_a_global_cap_across_sessions() {
    let mut rows = VecDeque::new();
    for pid in 1..=65 {
        remember_finished(&mut rows, entry(&format!("session-{pid}"), pid));
    }
    assert_eq!(rows.len(), 64);
    assert_eq!(rows.front().unwrap().process.pid, 2);
    assert_eq!(rows.back().unwrap().process.pid, 65);
}

#[test]
fn handoff_merges_and_caps_history_then_deletion_clears_only_its_session() {
    let mut rows = VecDeque::new();
    for pid in 1..=7 {
        remember_finished(&mut rows, entry("old", pid));
        remember_finished(&mut rows, entry("new", pid + 100));
    }
    remember_finished(&mut rows, entry("other", 999));
    handoff_finished(&mut rows, "old", "new");
    assert_eq!(rows.iter().filter(|s| s.session_id == "new").count(), 8);
    assert!(rows.iter().any(|s| s.session_id == "other"));
    forget_finished(&mut rows, "new");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows.front().unwrap().session_id, "other");
}
