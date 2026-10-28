//! Output batching and flow-control checks without a PTY or window.
//! Depends on `flow` and std channels/threads. Called by the crate's unit-test harness.

use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use crate::flow::Output;
use crate::types::AttachInfo;
use crate::viewer::{FLUSH_WINDOW, Viewer};

const CAP: usize = 64 * 1024;
const WINDOW: Duration = Duration::from_millis(8);
const WAIT: Duration = Duration::from_secs(2);

fn output(high_water: u64, timeout: Duration) -> Arc<Output> {
    Arc::new(Output::new(4 * CAP, high_water, timeout).unwrap())
}

fn attach(output: &Output) -> (AttachInfo, mpsc::Receiver<Vec<u8>>) {
    let (tx, rx) = mpsc::channel();
    let info = output.attach(Box::new(move |bytes| tx.send(bytes.to_vec()).is_ok()));
    (info, rx)
}

#[test]
fn a_burst_merges_small_reads_and_flushes_at_the_cap() {
    // A fixed clock keeps scheduler pauses from turning this burst into separate idle reads.
    let (tx, rx) = mpsc::channel();
    let mut viewer = Viewer::new(1, Box::new(move |data| tx.send(data.to_vec()).is_ok()));
    let now = Instant::now();
    assert!(viewer.push(b"first", now));
    assert_eq!(rx.try_recv().unwrap(), b"first", "idle echo is immediate");
    for _ in 0..64 {
        assert!(viewer.push(&[b'x'; 1024], now));
    }
    let batch = rx.try_recv().unwrap();
    assert_eq!(batch.len(), CAP);
    assert!(batch.iter().all(|byte| *byte == b'x'));
    assert!(rx.try_recv().is_err());
}

#[test]
fn a_trailing_key_echo_flushes_without_another_read() {
    let output = output(16 * CAP as u64, WAIT);
    let (_, rx) = attach(&output);
    output.push(b"a");
    assert_eq!(rx.try_recv().unwrap(), b"a");
    let started = Instant::now();
    output.push(b"b");
    assert_eq!(rx.recv_timeout(WAIT).unwrap(), b"b");
    // The deadline is 8 ms; allow scheduling tolerance on loaded build/test machines.
    assert!(started.elapsed() < Duration::from_millis(250));
    thread::sleep(2 * WINDOW);
    output.push(b"c");
    assert_eq!(rx.try_recv().unwrap(), b"c", "echo after idle is immediate");
}

#[test]
fn batches_preserve_order_and_exact_bytes_across_caps_and_a_tail() {
    let output = output(16 * CAP as u64, WAIT);
    let (_, rx) = attach(&output);
    let bytes: Vec<u8> = (0_u8..251).cycle().take(3 * CAP + 317).collect();
    for chunk in bytes.chunks(997) {
        output.push(chunk);
    }
    let mut got = Vec::new();
    while got.len() < bytes.len() {
        let chunk = rx.recv_timeout(WAIT).unwrap();
        assert!(chunk.len() <= CAP);
        got.extend_from_slice(&chunk);
    }
    assert_eq!(got, bytes);
    assert_eq!(output.end(), bytes.len() as u64);
    assert!(rx.try_recv().is_err());
}

#[test]
fn replacement_replays_pending_bytes_once_before_live_output() {
    let output = output(16 * CAP as u64, WAIT);
    let (old, old_rx) = attach(&output);
    output.push(b"a");
    assert_eq!(old_rx.try_recv().unwrap(), b"a");
    output.push(b"bc");
    assert_eq!(output.read(Some(0), 100, false).text, "abc");
    let (new, rx) = attach(&output);
    assert_eq!((new.start, new.end), (0, 3));
    assert_eq!(rx.try_recv().unwrap(), b"abc");
    output.detach(old.viewer);
    output.ack(old.viewer, u64::MAX);
    output.push(b"def");
    assert_eq!(rx.recv_timeout(WAIT).unwrap(), b"def");
    assert!(rx.try_recv().is_err());
    // Bytes already sent before replacement belong to the old attachment; none follow it.
    let _ = old_rx.try_iter().count();
    assert!(old_rx.recv_timeout(2 * WINDOW).is_err());
}

#[test]
fn high_water_flushes_pending_bytes_then_waits_for_the_matching_ack() {
    let output = output(4, WAIT);
    let (viewer, rx) = attach(&output);
    output.push(b"a");
    assert_eq!(rx.try_recv().unwrap(), b"a");
    let (done_tx, done_rx) = mpsc::channel();
    let writer = output.clone();
    let pushed = thread::spawn(move || {
        writer.push(b"bcd");
        done_tx.send(()).unwrap();
    });
    assert_eq!(rx.recv_timeout(WAIT).unwrap(), b"bcd");
    assert!(done_rx.recv_timeout(2 * WINDOW).is_err());
    output.ack(viewer.viewer + 1, 4);
    assert!(done_rx.recv_timeout(2 * WINDOW).is_err());
    output.ack(viewer.viewer, 4);
    done_rx.recv_timeout(WAIT).unwrap();
    pushed.join().unwrap();
}

#[test]
fn ack_timeout_detaches_and_hidden_output_keeps_buffering() {
    let timeout = Duration::from_millis(40);
    let output = output(4, timeout);
    let (_, rx) = attach(&output);
    let started = Instant::now();
    output.push(b"abcd");
    assert!(started.elapsed() >= timeout);
    assert_eq!(rx.recv_timeout(WAIT).unwrap(), b"abcd");
    assert!(
        rx.recv_timeout(WAIT).is_err(),
        "timed-out viewer is dropped"
    );
    output.push(b"efgh");
    assert_eq!(output.read(Some(0), 100, false).text, "abcdefgh");
}

#[test]
fn closing_releases_a_reader_waiting_for_ack() {
    let output = output(4, WAIT);
    let (_, rx) = attach(&output);
    let writer = output.clone();
    let pushed = thread::spawn(move || writer.push(b"abcd"));
    assert_eq!(rx.recv_timeout(WAIT).unwrap(), b"abcd");
    output.close();
    pushed.join().unwrap();
    output.push(b"tail");
    assert_eq!(output.read(Some(0), 100, false).text, "abcdtail");
}

#[test]
fn restart_notes_follow_pending_output_and_are_acknowledged() {
    let output = output(4, WAIT);
    let (viewer, rx) = attach(&output);
    output.push(b"a");
    assert_eq!(rx.try_recv().unwrap(), b"a");
    output.push(b"b");
    output.note(b"\r\n");
    let mut got = Vec::new();
    while got.len() < 3 {
        got.extend(rx.recv_timeout(WAIT).unwrap());
    }
    assert_eq!(got, b"b\r\n");
    let (done_tx, done_rx) = mpsc::channel();
    let writer = output.clone();
    let pushed = thread::spawn(move || {
        writer.push(b"c");
        done_tx.send(()).unwrap();
    });
    assert_eq!(rx.recv_timeout(WAIT).unwrap(), b"c");
    assert!(done_rx.recv_timeout(2 * WINDOW).is_err());
    output.ack(viewer.viewer, 5);
    done_rx.recv_timeout(WAIT).unwrap();
    pushed.join().unwrap();
}

#[test]
fn replay_and_large_pushes_respect_the_message_cap() {
    let output = output(16 * CAP as u64, WAIT);
    let data = vec![b'x'; 2 * CAP + 3];
    output.push(&data);
    let (viewer, rx) = attach(&output);
    let mut got = Vec::new();
    while got.len() < data.len() {
        let chunk = rx.recv_timeout(WAIT).unwrap();
        assert!(chunk.len() <= CAP, "oversized replay: {}", chunk.len());
        got.extend(chunk);
    }
    assert_eq!(got, data);
    assert_eq!(viewer.end, data.len() as u64);
    output.push(&data);
    let mut got = Vec::new();
    while got.len() < data.len() {
        let chunk = rx.recv_timeout(WAIT).unwrap();
        assert!(chunk.len() <= CAP, "oversized live output: {}", chunk.len());
        got.extend(chunk);
    }
    assert_eq!(got, data);
}

#[test]
fn detach_discards_pending_delivery_but_keeps_the_scrollback() {
    let output = output(16 * CAP as u64, WAIT);
    let (viewer, rx) = attach(&output);
    output.push(b"a");
    assert_eq!(rx.try_recv().unwrap(), b"a");
    output.push(b"b");
    output.detach(viewer.viewer);
    let _ = rx.try_iter().count();
    assert!(rx.recv_timeout(WAIT).is_err());
    output.push(b"c");
    let (_, rx) = attach(&output);
    assert_eq!(rx.recv_timeout(WAIT).unwrap(), b"abc");
}

#[test]
fn dropping_the_output_stops_the_timer_and_drops_its_sink() {
    let output = output(16 * CAP as u64, WAIT);
    let (_, rx) = attach(&output);
    output.push(b"a");
    assert_eq!(rx.try_recv().unwrap(), b"a");
    output.push(b"b");
    drop(output);
    let _ = rx.try_iter().count();
    assert!(rx.recv_timeout(WAIT).is_err());
}

#[test]
fn later_reads_do_not_extend_the_first_pending_bytes_deadline() {
    let (tx, rx) = mpsc::channel();
    let mut viewer = Viewer::new(1, Box::new(move |data| tx.send(data.to_vec()).is_ok()));
    let now = Instant::now();
    assert!(viewer.push(b"a", now));
    assert_eq!(rx.try_recv().unwrap(), b"a");
    assert!(viewer.push(b"b", now + Duration::from_millis(1)));
    let due = now + Duration::from_millis(1) + FLUSH_WINDOW;
    assert_eq!(viewer.deadline, Some(due));
    assert!(viewer.push(b"c", now + FLUSH_WINDOW));
    assert_eq!(viewer.deadline, Some(due));
    assert!(rx.try_recv().is_err());
    assert!(viewer.flush(due));
    assert_eq!(rx.try_recv().unwrap(), b"bc");
    assert_eq!(viewer.deadline, None);
    assert!(viewer.push(b"d", due + FLUSH_WINDOW));
    assert_eq!(rx.try_recv().unwrap(), b"d");
}

#[test]
fn a_failed_timed_send_detaches_its_sink_and_keeps_buffering() {
    let output = output(16 * CAP as u64, WAIT);
    let (tx, rx) = mpsc::channel();
    let alive = Arc::new(());
    let weak = Arc::downgrade(&alive);
    output.attach(Box::new(move |bytes| {
        let _ = &alive;
        tx.send(bytes.to_vec()).is_ok()
    }));
    output.push(b"a");
    assert_eq!(rx.try_recv().unwrap(), b"a");
    drop(rx);
    output.push(b"b");
    let deadline = Instant::now() + WAIT;
    while weak.upgrade().is_some() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(1));
    }
    assert!(weak.upgrade().is_none(), "failed sink was not detached");
    output.push(b"c");
    let (_, rx) = attach(&output);
    assert_eq!(rx.recv_timeout(WAIT).unwrap(), b"abc");
}
