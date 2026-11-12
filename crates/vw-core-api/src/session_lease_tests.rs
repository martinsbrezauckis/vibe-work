//! Exercises lease release ordering and independent claims with real OS locks.
//! Depends on its parent lease owner and test-only rendezvous channels; called by core unit tests.
use super::*;

fn released_lock(leases: &Leases, id: &str) -> File {
    let lock = leases.lock(id).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while let Err(error) = lock.try_lock() {
        assert!(
            matches!(error, TryLockError::WouldBlock) && std::time::Instant::now() < deadline,
            "the released {id} lock did not come free: {error:?}"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    lock
}

/// A claim racing this instance's own lease release (the lock file still open for a moment after the last
/// handle dropped) must wait for it, never report the session as held by another window with our own pid.
#[test]
fn a_claim_during_our_own_release_waits_instead_of_refusing() {
    let dir = std::env::temp_dir().join(format!("vw-lease-race-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let leases = Leases::new(&dir.join("home"), Some(&dir.join("data"))).unwrap();
    drop(leases.claim("s1").unwrap());
    // The released lease's lock file is still open: hold the OS lock the way its closing handle does. A child
    // process another test starts at this moment can share the released lock until it runs its own program, so
    // the setup waits (at most 5 s) for the lock to come free first.
    let closing = released_lock(&leases, "s1");
    assert_eq!(
        leases.foreign("s1").unwrap(),
        None,
        "our own release is not another window"
    );
    let release = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(20));
        drop(closing);
    });
    assert!(
        leases.claim("s1").unwrap().is_some(),
        "the claim waits for our release"
    );
    release.join().unwrap();

    // Another instance (another window sharing the Vibe home) is still refused at once.
    let other = Leases::new(&dir.join("home"), Some(&dir.join("other-data"))).unwrap();
    let held = leases.claim("s2").unwrap();
    let refused = other.claim("s2").err().expect("held by the first instance");
    assert_eq!(refused.server_code.as_deref(), Some("sessionHeld"));
    assert_eq!(other.foreign("s2").unwrap(), Some(std::process::id()));
    drop(held);
    let _ = std::fs::remove_dir_all(&dir);
}

pub(super) struct Pause {
    reached: std::sync::mpsc::Sender<()>,
    resume: std::sync::mpsc::Receiver<()>,
}

pub(super) fn pause(hook: &Mutex<Option<Pause>>) {
    let pause = hook.lock().unwrap().take();
    if let Some(pause) = pause {
        pause.reached.send(()).unwrap();
        pause.resume.recv().unwrap();
    }
}

#[test]
fn a_fork_release_wait_does_not_block_an_unrelated_claim() {
    use std::sync::mpsc;
    use std::time::Duration;
    let dir = std::env::temp_dir().join(format!("vw-lease-independent-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let leases = Arc::new(Leases::new(&dir.join("home"), Some(&dir.join("data"))).unwrap());
    drop(leases.claim("waiting").unwrap());
    let closing = released_lock(&leases, "waiting");
    let (reached_tx, reached_rx) = mpsc::channel();
    let (resume_tx, resume_rx) = mpsc::channel();
    *leases.before_wait.lock().unwrap() = Some(Pause {
        reached: reached_tx,
        resume: resume_rx,
    });
    let waiting = {
        let leases = leases.clone();
        std::thread::spawn(move || leases.claim("waiting"))
    };
    reached_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let (claimed_tx, claimed_rx) = mpsc::channel();
    let independent = {
        let leases = leases.clone();
        std::thread::spawn(move || claimed_tx.send(leases.claim("independent")).unwrap())
    };
    let progressed = claimed_rx.recv_timeout(Duration::from_secs(5));
    // Always release both threads before asserting, including the red run.
    drop(closing);
    resume_tx.send(()).unwrap();
    drop(waiting.join().unwrap().unwrap());
    independent.join().unwrap();
    assert!(
        progressed.is_ok(),
        "another chat's claim was blocked by the fork-release wait"
    );
    drop(progressed.unwrap().unwrap());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_failed_pointer_write_releases_the_os_lock_and_registry() {
    let dir = std::env::temp_dir().join(format!("vw-lease-failed-write-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let leases = Leases::new(&dir.join("home"), Some(&dir.join("data"))).unwrap();
    let pointer = leases.pointer("retry");
    std::fs::create_dir_all(&pointer).unwrap();
    assert!(leases.claim("retry").is_err());
    std::fs::remove_dir(&pointer).unwrap();
    let released = released_lock(&leases, "retry");
    released.unlock().unwrap();
    drop(released);
    let held = leases.claim("retry").unwrap().unwrap();
    assert!(Arc::ptr_eq(&held, &leases.claim("retry").unwrap().unwrap()));
    drop(held);
    // Parallel tests can fork while the lease is held. Wait for those inherited handles to exec,
    // then explicitly unlock the probe (also releases any forked duplicate of this probe itself).
    let released = released_lock(&leases, "retry");
    released.unlock().unwrap();
    drop(released);
    let other = Leases::new(&dir.join("home"), Some(&dir.join("other"))).unwrap();
    drop(other.claim("retry").unwrap());
    let _ = std::fs::remove_dir_all(dir);
}

#[cfg(unix)]
#[test]
fn ownership_probes_release_a_lock_inherited_by_a_fork() {
    let dir = tempfile::tempdir().unwrap();
    let leases = Leases::new(&dir.path().join("home"), Some(&dir.path().join("data"))).unwrap();
    let lock = leases.lock("s1").unwrap();
    let inherited = lock.try_clone().unwrap();
    probe(lock).unwrap();
    assert!(
        leases.claim("s1").is_ok(),
        "a completed ownership probe must not leave an inherited lock behind"
    );
    drop(inherited);
}

#[cfg(unix)]
#[test]
fn releasing_a_lease_frees_a_lock_inherited_by_a_fork() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let first = Leases::new(&home, Some(&dir.path().join("first"))).unwrap();
    let next = Leases::new(&home, Some(&dir.path().join("next"))).unwrap();
    let lease = first.claim("s1").unwrap().unwrap();
    // A fork inherits the same open file description until exec closes its descriptor.
    let inherited = lease.lock.as_ref().unwrap().try_clone().unwrap();
    let refused = next
        .claim("s1")
        .err()
        .expect("the live owner keeps its lease");
    assert_eq!(refused.server_code.as_deref(), Some("sessionHeld"));
    first.retain("s1", Some(lease));
    first.release_all();
    let next_owner = next
        .claim("s1")
        .expect("a shut-down owner must release its lease before an inherited descriptor closes");
    drop(inherited);
    let refused = first
        .claim("s1")
        .err()
        .expect("the new owner keeps its lease");
    assert_eq!(refused.server_code.as_deref(), Some("sessionHeld"));
    drop(next_owner);
}
