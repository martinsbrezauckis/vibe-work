//! Deterministic delayed-lookup and PID-reuse regressions for authenticated process registration.
//! Uses a separately owned child and a saturated blocking pool; no OS PID churn is needed.

use super::*;

struct Unrelated(std::process::Child);
#[allow(clippy::unwrap_used)]
impl Unrelated {
    fn start() -> Self {
        Self(
            std::process::Command::new("sleep")
                .arg("30")
                .spawn()
                .unwrap(),
        )
    }
    fn identity(&self) -> ProcessRef {
        vw_procmon::identity(self.0.id()).unwrap()
    }
}
impl Drop for Unrelated {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn receipt(process: ProcessRef) -> Receipt {
    Receipt {
        session: "late".into(),
        id: "old".into(),
        command: "former child".into(),
        pid: process.pid,
        start_token: process.start.0,
    }
}

#[test]
fn late_blocking_lookup_cannot_register_a_reused_pid_after_timeout() {
    for reused in [false, true] {
        let monitor = vw_procmon::ProcMon::new(vw_procmon::ProcMonConfig::default()).unwrap();
        let records = Records::default();
        let unrelated = Unrelated::start();
        let current = unrelated.identity();
        let mut observed = current;
        if reused {
            // The launcher's former child had the same PID, but a different token.
            observed.start.0 -= 1;
        }
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .max_blocking_threads(1)
            .build()
            .unwrap();
        runtime.block_on(async {
            let (started, ready) = tokio::sync::oneshot::channel();
            let (release, blocked) = std::sync::mpsc::channel();
            let occupied = tokio::task::spawn_blocking(move || {
                started.send(()).unwrap();
                blocked.recv().unwrap();
            });
            ready.await.unwrap();
            // receive's timeout drops its future with identity work still queued.
            // The launcher may now stop/reap its child and the OS reuse the PID.
            let work = register(
                receipt(observed),
                records.clone(),
                monitor.registry().clone(),
                vw_procmon::identity,
            );
            assert!(
                tokio::time::timeout(Duration::from_millis(50), work)
                    .await
                    .is_err()
            );
            release.send(()).unwrap();
            occupied.await.unwrap();
        });
        // Drain detached blocking tasks before checking: this does not race publication.
        runtime.shutdown_timeout(Duration::from_secs(5));
        assert!(
            monitor.registry().list().is_empty(),
            "late lookup registered a process after timeout (reused={reused})"
        );
        assert!(records.lock().unwrap().is_empty());
        assert!(vw_procmon::is_alive(current));
    }
}

#[tokio::test]
async fn reused_pid_is_refused_even_while_the_receiver_is_waiting() {
    let monitor = vw_procmon::ProcMon::new(vw_procmon::ProcMonConfig::default()).unwrap();
    let records = Records::default();
    let unrelated = Unrelated::start();
    let current = unrelated.identity();
    let mut observed = current;
    observed.start.0 -= 1;
    let registered = register(
        receipt(observed),
        records.clone(),
        monitor.registry().clone(),
        vw_procmon::identity,
    )
    .await
    .unwrap();
    assert!(
        !registered,
        "the replacement's start token became an ownership fact"
    );
    assert!(records.lock().unwrap().is_empty());
    assert_eq!(monitor.registry().list(), []);
    assert!(vw_procmon::is_alive(current));
}

#[cfg(target_os = "linux")]
#[test]
fn launcher_and_core_read_the_same_native_start_token() {
    let unrelated = Unrelated::start();
    let current = unrelated.identity();
    let script = concat!(
        include_str!("../../vw-supervisor/src/vibe_process_identity.py"),
        "\nimport sys\nprint(native_process_start_token(int(sys.argv[1])))"
    );
    let output = std::process::Command::new("python3")
        .args(["-I", "-c", script, &current.pid.to_string()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let token = String::from_utf8(output.stdout)
        .unwrap()
        .trim()
        .parse::<u64>()
        .unwrap();
    assert_eq!(token, current.start.0);
}
