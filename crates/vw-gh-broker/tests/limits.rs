//! The broker answers each caller on its own, stops a gh run that takes too long, and refuses `gh api --verbose`.
//! Uses the fake gh (its `sleep-ms` cwd file makes a run slow) and a fabricated token.
//!
//! PREDICT (written before the change): the broker answered one connection at a time, so a quick read waits behind
//! a 3-second one (the "answered while the slow run is busy" check fails); `api --verbose` reaches gh (exit 0,
//! `runs` written) instead of being refused; a gh run never ends early (there was no time limit to set).

#![allow(clippy::unwrap_used)]

mod support;

use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
use support::Harness;
use vw_gh_broker::{Request, call};

fn stop(harness: &Harness, stopped: &AtomicBool) {
    stopped.store(true, Ordering::Release);
    // One more connection wakes the accept so the serving loop sees the stop.
    let _ = call(
        harness.broker.endpoint(),
        &Request {
            argv: Vec::new(),
            cwd: std::env::temp_dir(),
        },
    );
}

#[test]
fn a_slow_read_does_not_hold_other_callers() {
    let harness = Harness::new();
    std::fs::write(harness.cwd.path().join("sleep-ms"), "3000").unwrap();
    let quick = tempfile::tempdir().unwrap();
    let stopped = AtomicBool::new(false);
    std::thread::scope(|scope| {
        let server = scope.spawn(|| harness.broker.serve_until(&stopped).unwrap());
        let slow = scope.spawn(|| {
            call(
                harness.broker.endpoint(),
                &harness.request(&["repo", "view", "owner/slow"]),
            )
            .unwrap()
        });
        // Give the slow run time to start before the quick one connects.
        std::thread::sleep(Duration::from_millis(500));
        let started = Instant::now();
        let reply = call(
            harness.broker.endpoint(),
            &Request {
                argv: vec!["repo".into(), "view".into(), "owner/quick".into()],
                cwd: quick.path().to_owned(),
            },
        )
        .unwrap();
        let waited = started.elapsed();
        let slow_busy = !slow.is_finished();
        let slow_reply = slow.join().unwrap();
        // Stop serving before any assertion, so a failure never leaves the scope waiting on accept.
        stop(&harness, &stopped);
        assert_eq!(reply.exit_code, 0);
        assert!(
            slow_busy,
            "the quick read was answered while the slow run was busy (waited {waited:?})"
        );
        assert!(waited < Duration::from_millis(2000), "waited {waited:?}");
        assert_eq!(slow_reply.exit_code, 0);
        server.join().unwrap();
    });
}

#[test]
fn api_verbose_is_refused_before_gh_runs() {
    let harness = Harness::new();
    for argv in [
        &["api", "repos/o/r", "--verbose"][..],
        &["api", "--verbose", "user"][..],
    ] {
        let reply = harness.call(argv);
        assert_eq!(reply.exit_code, 1, "accepted {argv:?}");
        assert!(!harness.cwd.path().join("runs").exists(), "{argv:?} ran gh");
    }
    // `gh run view --verbose` is a different, still allowed flag.
    assert_eq!(
        harness.call(&["run", "view", "1", "--verbose"]).exit_code,
        0
    );
}

#[test]
fn a_run_past_the_time_limit_is_stopped_and_refused() {
    let harness = Harness {
        broker: harness_broker().with_time_limit(Duration::from_secs(1)),
        cwd: tempfile::tempdir().unwrap(),
    };
    std::fs::write(harness.cwd.path().join("sleep-ms"), "30000").unwrap();
    let started = Instant::now();
    let reply = harness.call(&["repo", "view", "owner/stalled"]);
    let took = started.elapsed();
    assert!(took < Duration::from_secs(10), "took {took:?}");
    assert_eq!(reply.exit_code, 1);
    let stderr = String::from_utf8_lossy(&reply.stderr);
    assert!(
        stderr.contains("did not finish within 1 seconds") && stderr.contains("try again"),
        "{stderr}"
    );
    // The broker keeps serving after a stopped run.
    std::fs::remove_file(harness.cwd.path().join("sleep-ms")).unwrap();
    assert_eq!(harness.call(&["repo", "view", "owner/next"]).exit_code, 0);
}

fn harness_broker() -> vw_gh_broker::Broker {
    vw_gh_broker::Broker::bind(support::TOKEN.to_owned(), support::fake_gh()).unwrap()
}
