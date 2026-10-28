//! Hostile receipt reads in Tools inventory. Called by vw-market's tests; uses disposable files
//! and Store's real reader to check size and special-file rejection without leaving blocked readers.

use super::{RECEIPT, Receipt, Store};
use std::path::PathBuf;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        // A counter, not only the clock: macOS's clock ticks in microseconds, so parallel tests got one folder and
        // the first to finish removed the other's files.
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let home = std::env::temp_dir().join(format!(
            "vw-receipt-read-test-{}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(home.join("plugins/candidate")).unwrap();
        Self(home)
    }

    fn receipt(&self) -> PathBuf {
        self.0.join("plugins/candidate").join(RECEIPT)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn bytes() -> Vec<u8> {
    serde_json::to_vec(&Receipt {
        entry_id: "candidate".into(),
        version: "1".into(),
        catalog_version: "1".into(),
        installed_at_ms: 1,
        files: std::collections::BTreeMap::new(),
    })
    .unwrap()
}

#[test]
fn oversized_receipt_is_unreadable() {
    let f = Fixture::new();
    let mut bytes = bytes();
    bytes.resize(1024 * 1024 + 1, b' ');
    std::fs::write(f.receipt(), bytes).unwrap();
    assert!(Store::new(&f.0).receipt("candidate").is_err());
}

#[test]
fn a_receipt_at_the_byte_limit_is_readable() {
    let f = Fixture::new();
    let mut bytes = bytes();
    bytes.resize(1024 * 1024, b' ');
    std::fs::write(f.receipt(), bytes).unwrap();
    assert!(Store::new(&f.0).receipt("candidate").unwrap().is_some());
}

#[cfg(unix)]
#[test]
fn linked_receipt_is_unreadable() {
    let f = Fixture::new();
    let target = f.0.join("receipt.json");
    std::fs::write(&target, bytes()).unwrap();
    std::os::unix::fs::symlink(target, f.receipt()).unwrap();
    assert!(Store::new(&f.0).receipt("candidate").is_err());
}

#[cfg(unix)]
#[test]
fn fifo_receipt_is_refused_without_waiting_for_a_writer() {
    use std::io::Write as _;
    use std::time::Duration;
    let f = Fixture::new();
    assert!(
        std::process::Command::new("mkfifo")
            .arg(f.receipt())
            .status()
            .unwrap()
            .success()
    );
    let store = Store::new(&f.0);
    let (send, receive) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || send.send(store.receipt("candidate")).unwrap());
    let early = receive.recv_timeout(Duration::from_millis(200));
    let timely = early.is_ok();
    let result = early.unwrap_or_else(|_| {
        let mut writer = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(f.receipt())
            .unwrap();
        writer.write_all(&bytes()).unwrap();
        drop(writer);
        receive.recv_timeout(Duration::from_secs(2)).unwrap()
    });
    reader.join().unwrap();
    assert!(timely, "receipt blocked waiting for a FIFO writer");
    assert!(result.is_err());
}
