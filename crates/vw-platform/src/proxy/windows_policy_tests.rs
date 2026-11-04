//! Decision-table regressions for the native Windows adapter; all run without `WinHTTP` on Linux.
//! Fake lookup/fallback closures check selection, failure boundaries and repeat-connection latency.
use super::*;
use std::cell::Cell;

fn manual(set: bool) -> Result<Vec<String>, &'static str> {
    if set {
        crate::proxy::manual::routes("https://example.test/", "proxy.test:8080", "")
    } else {
        Ok(vec!["direct://".into()])
    }
}

#[test]
fn automatic_settings_and_outcomes_select_routes() {
    let found = Ok(vec!["http://pac.test:80".into(), "direct://".into()]);
    // SDK errors: no WPAD, WPAD DNS failure, PAC download failure (including 404), timeout.
    let outcomes = [
        (found.clone(), false),
        (Err(Failure::WinHttp(12180)), true),
        (Err(Failure::WinHttp(12007)), true),
        (Err(Failure::WinHttp(12167)), true),
        (Err(Failure::WinHttp(12002)), true),
        (Err(Failure::Timeout), true),
        (Err(Failure::WinHttp(12166)), false), // downloaded PAC failed to evaluate
        (Err(Failure::WinHttp(12178)), false), // resolver service unavailable
        (Err(Failure::WinHttp(12015)), false), // authentication required
        (Err(Failure::WinHttp(5)), false),     // access denied
        (Err(Failure::Internal), false),
    ];
    for auto_detect in [false, true] {
        for pac_url in [false, true] {
            for manual_set in [false, true] {
                for (outcome, absent) in &outcomes {
                    let mode = Mode::from_settings(auto_detect, pac_url);
                    let expected = if !pac_url && (!auto_detect || *absent) {
                        manual(manual_set)
                    } else {
                        outcome.clone().map_err(|_| FAILED)
                    };
                    assert_eq!(
                        Policy::default().resolve(mode, || outcome.clone(), || manual(manual_set)),
                        expected,
                        "auto={auto_detect}, pac={pac_url}, manual={manual_set}, outcome={outcome:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn negative_discovery_skips_repeat_lookup_but_reads_current_manual_routes() {
    let policy = Policy::default();
    let calls = Cell::new(0);
    for manual_set in [false, true, false] {
        assert_eq!(
            policy.resolve(
                Mode::Discover,
                || {
                    calls.set(calls.get() + 1);
                    Err(Failure::WinHttp(12180))
                },
                || manual(manual_set),
            ),
            manual(manual_set)
        );
    }
    assert_eq!(calls.get(), 1, "WPAD must not run on every connection");
}

#[test]
fn every_discovery_miss_is_cached_and_expires() {
    for error in [
        Failure::WinHttp(12180),
        Failure::WinHttp(12007),
        Failure::WinHttp(12167),
        Failure::WinHttp(12002),
        Failure::Timeout,
    ] {
        let policy = Policy::default();
        assert_eq!(
            policy.resolve(Mode::Discover, || Err(error), || manual(false)),
            manual(false)
        );
        let deadline = policy.absent_until.lock().unwrap().unwrap();
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(remaining <= Duration::from_secs(300));
        assert!(remaining > Duration::from_secs(290));
        assert_eq!(
            policy.resolve(Mode::Discover, || panic!("cached miss"), || manual(true)),
            manual(true)
        );
        *policy.absent_until.lock().unwrap() = Some(Instant::now());
        assert_eq!(
            policy.resolve(
                Mode::Discover,
                || Ok(vec!["http://new-network:80".into()]),
                || panic!("PAC found")
            ),
            Ok(vec!["http://new-network:80".into()])
        );
    }
}

#[test]
fn configured_pac_ignores_and_clears_negative_cache() {
    let policy = Policy::default();
    for outcome in [
        Ok(vec!["http://configured:80".into()]),
        Err(Failure::WinHttp(12167)),
        Err(Failure::Timeout),
    ] {
        assert_eq!(
            policy.resolve(
                Mode::Discover,
                || Err(Failure::WinHttp(12180)),
                || manual(false)
            ),
            manual(false)
        );
        assert_eq!(
            policy.resolve(
                Mode::PacUrl,
                || outcome.clone(),
                || panic!("configured PAC must not fall back")
            ),
            outcome.map_err(|_| FAILED)
        );
        assert!(policy.absent_until.lock().unwrap().is_none());
    }
}

#[test]
fn disabling_discovery_clears_cache_and_manual_bypass_is_preserved() {
    let policy = Policy::default();
    let bypass = || crate::proxy::manual::routes("https://inside.corp/", "proxy:80", "*.corp");
    assert_eq!(
        policy.resolve(Mode::Discover, || Err(Failure::WinHttp(12180)), bypass),
        manual(false)
    );
    assert_eq!(
        policy.resolve(Mode::Manual, || panic!("disabled"), bypass),
        manual(false)
    );
    assert!(policy.absent_until.lock().unwrap().is_none());
    assert_eq!(
        policy.resolve(
            Mode::Discover,
            || Ok(vec!["http://pac:80".into()]),
            || panic!("PAC found")
        ),
        Ok(vec!["http://pac:80".into()])
    );
}

#[test]
fn successful_per_url_routes_and_unexpected_errors_are_not_cached() {
    let policy = Policy::default();
    for outcome in [
        Ok(vec!["http://first:80".into()]),
        Err(Failure::Internal),
        Ok(vec!["direct://".into()]),
        Err(Failure::WinHttp(12166)),
    ] {
        assert_eq!(
            policy.resolve(Mode::Discover, || outcome.clone(), || panic!("no fallback")),
            outcome.map_err(|_| FAILED)
        );
        assert!(policy.absent_until.lock().unwrap().is_none());
    }
}

#[test]
fn fallback_failure_remains_closed_including_with_a_cached_miss() {
    let policy = Policy::default();
    for _ in 0..2 {
        assert_eq!(
            policy.resolve(
                Mode::Discover,
                || Err(Failure::WinHttp(12180)),
                || Err("default settings unavailable")
            ),
            Err("default settings unavailable")
        );
    }
}

#[test]
fn concurrent_connections_use_cached_negative_discovery() {
    use std::sync::{
        Barrier,
        atomic::{AtomicUsize, Ordering},
    };
    let policy = Policy::default();
    assert_eq!(
        policy.resolve(Mode::Discover, || Err(Failure::Timeout), || manual(false)),
        manual(false)
    );
    let calls = AtomicUsize::new(0);
    let barrier = Barrier::new(8);
    std::thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| {
                barrier.wait();
                assert_eq!(
                    policy.resolve(
                        Mode::Discover,
                        || {
                            calls.fetch_add(1, Ordering::SeqCst);
                            Err(Failure::Timeout)
                        },
                        || manual(false)
                    ),
                    manual(false)
                );
            });
        }
    });
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn per_url_pac_lookups_do_not_serialize_behind_the_cache_lock() {
    use std::sync::mpsc;
    let policy = Policy::default();
    let (entered, waiting) = mpsc::channel();
    let (release, released) = mpsc::channel();
    std::thread::scope(|scope| {
        let first = scope.spawn(|| {
            policy.resolve(
                Mode::Discover,
                move || {
                    entered.send(()).unwrap();
                    // A second native lookup must run while this one waits for its own PAC answer.
                    released.recv_timeout(Duration::from_secs(2)).unwrap();
                    Ok(vec!["http://first:80".into()])
                },
                || panic!("no fallback"),
            )
        });
        waiting.recv_timeout(Duration::from_secs(2)).unwrap();
        let second = policy.resolve(
            Mode::Discover,
            || {
                let _ = release.send(());
                Ok(vec!["http://second:80".into()])
            },
            || panic!("no fallback"),
        );
        assert_eq!(first.join().unwrap(), Ok(vec!["http://first:80".into()]));
        assert_eq!(second, Ok(vec!["http://second:80".into()]));
    });
}
