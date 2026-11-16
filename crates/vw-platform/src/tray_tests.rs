//! The tray host watch against a private session bus (`dbus-daemon`, started per test) and a fake
//! `org.kde.StatusNotifierWatcher`. Needs the `dbus-daemon` program; without it the test prints NOT RUN, and fails
//! under `VW_REQUIRE_ALL=1`.
//!
//! PREDICT: the watch opened a new session-bus connection for every probe (each signal and each one-second check),
//! so over about three seconds it used four or more connections; it now opens one and probes over it, so the count
//! stays at one while the answers (host present, host lost by signal, watcher gone) stay the same.
//! PREDICT (host cache): there was no stored answer, so the Settings row probed the bus on every read; the cache case
//! does not compile without [`super::HostCache`]. Fed by the watch, it reports each coming and going once (not every
//! one-second check) and answers reads without a bus connection.
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use tokio::sync::mpsc;

struct Bus {
    daemon: Child,
    address: String,
}

impl Drop for Bus {
    fn drop(&mut self) {
        let _ = self.daemon.kill();
        let _ = self.daemon.wait();
    }
}

fn private_bus() -> Option<Bus> {
    use std::io::BufRead;
    let mut daemon = Command::new("dbus-daemon")
        .args(["--session", "--nofork", "--print-address=1"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut line = String::new();
    std::io::BufReader::new(daemon.stdout.take()?)
        .read_line(&mut line)
        .ok()?;
    Some(Bus {
        daemon,
        address: line.trim().to_owned(),
    })
}

struct Watcher {
    host: bool,
}

#[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
impl Watcher {
    #[zbus(property)]
    fn is_status_notifier_host_registered(&self) -> bool {
        self.host
    }

    #[zbus(signal)]
    async fn status_notifier_host_unregistered(
        emitter: &zbus::object_server::SignalEmitter<'_>,
    ) -> zbus::Result<()>;
}

async fn connect(address: &str) -> zbus::Result<zbus::Connection> {
    zbus::connection::Builder::address(address)?.build().await
}

/// The next answer within `within`, skipping repeats of `skip`.
async fn next(
    answers: &mut mpsc::UnboundedReceiver<bool>,
    skip: bool,
    within: Duration,
) -> Option<bool> {
    tokio::time::timeout(within, async {
        loop {
            match answers.recv().await {
                Some(answer) if answer == skip => {}
                other => return other,
            }
        }
    })
    .await
    .ok()
    .flatten()
}

#[tokio::test]
async fn one_connection_carries_the_signals_and_every_probe() {
    let Some(bus) = private_bus() else {
        eprintln!("NOT RUN: tray host watch test needs the dbus-daemon program");
        assert!(
            std::env::var("VW_REQUIRE_ALL").as_deref() != Ok("1"),
            "dbus-daemon is required under VW_REQUIRE_ALL=1"
        );
        return;
    };
    let watcher = zbus::connection::Builder::address(bus.address.as_str())
        .unwrap()
        .name("org.kde.StatusNotifierWatcher")
        .unwrap()
        .serve_at("/StatusNotifierWatcher", Watcher { host: true })
        .unwrap()
        .build()
        .await
        .unwrap();
    let opened = Arc::new(AtomicUsize::new(0));
    let (send, mut answers) = mpsc::unbounded_channel();
    let watch = {
        let (opened, address) = (opened.clone(), bus.address.clone());
        tokio::spawn(async move {
            super::watch_host_on(
                move || {
                    opened.fetch_add(1, Ordering::SeqCst);
                    let address = address.clone();
                    async move { connect(&address).await }
                },
                move |host| {
                    let _ = send.send(host);
                },
            )
            .await;
        })
    };
    assert_eq!(
        next(&mut answers, false, Duration::from_secs(3)).await,
        Some(true)
    );

    // The host goes away and the watcher says so: the watch answers false.
    let iface = watcher
        .object_server()
        .interface::<_, Watcher>("/StatusNotifierWatcher")
        .await
        .unwrap();
    iface.get_mut().await.host = false;
    Watcher::status_notifier_host_unregistered(iface.signal_emitter())
        .await
        .unwrap();
    assert_eq!(
        next(&mut answers, true, Duration::from_secs(3)).await,
        Some(false)
    );

    // A host registers without a signal: the one-second check still finds it.
    iface.get_mut().await.host = true;
    assert_eq!(
        next(&mut answers, false, Duration::from_secs(3)).await,
        Some(true)
    );

    // Let several one-second checks run, then the watcher leaves the bus.
    tokio::time::sleep(Duration::from_millis(2_500)).await;
    drop(iface);
    drop(watcher);
    assert_eq!(
        next(&mut answers, true, Duration::from_secs(3)).await,
        Some(false)
    );
    assert_eq!(
        opened.load(Ordering::SeqCst),
        1,
        "the watch opened a bus connection per probe"
    );
    watch.abort();
}

struct Other;

#[zbus::interface(name = "org.example.Unrelated")]
impl Other {
    #[zbus(signal)]
    async fn changed(emitter: &zbus::object_server::SignalEmitter<'_>) -> zbus::Result<()>;
}

/// True when nothing arrives on `signals` within 300 ms.
async fn quiet(
    signals: &mut (impl futures_util::Stream<Item = zbus::Result<zbus::Message>> + Unpin),
) -> bool {
    use futures_util::StreamExt;
    tokio::time::timeout(Duration::from_millis(300), signals.next())
        .await
        .is_err()
}

/// The subscription delivers the watcher's host-loss signal and its owner changes, and nothing else on the bus.
#[tokio::test]
async fn the_subscription_carries_only_the_two_watcher_signals() {
    use futures_util::StreamExt;
    let Some(bus) = private_bus() else {
        eprintln!("NOT RUN: tray host watch test needs the dbus-daemon program");
        assert!(
            std::env::var("VW_REQUIRE_ALL").as_deref() != Ok("1"),
            "dbus-daemon is required under VW_REQUIRE_ALL=1"
        );
        return;
    };
    let listener = connect(&bus.address).await.unwrap();
    let mut signals = super::watcher_signals(&listener).await.unwrap();
    let other = zbus::connection::Builder::address(bus.address.as_str())
        .unwrap()
        .serve_at("/other", Other)
        .unwrap()
        .build()
        .await
        .unwrap();
    let emitter = zbus::object_server::SignalEmitter::new(&other, "/other").unwrap();
    Other::changed(&emitter).await.unwrap();
    assert!(
        quiet(&mut signals).await,
        "an unrelated signal reached the watch"
    );
    let watcher = zbus::connection::Builder::address(bus.address.as_str())
        .unwrap()
        .name("org.kde.StatusNotifierWatcher")
        .unwrap()
        .serve_at("/StatusNotifierWatcher", Watcher { host: true })
        .unwrap()
        .build()
        .await
        .unwrap();
    let within = Duration::from_secs(2);
    let owner = tokio::time::timeout(within, signals.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        owner.header().member().unwrap().as_str(),
        "NameOwnerChanged"
    );
    let emitter =
        zbus::object_server::SignalEmitter::new(&watcher, "/StatusNotifierWatcher").unwrap();
    Watcher::status_notifier_host_unregistered(&emitter)
        .await
        .unwrap();
    let lost = tokio::time::timeout(within, signals.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        lost.header().member().unwrap().as_str(),
        "StatusNotifierHostUnregistered"
    );
    // Another name changing owner is not the watcher's.
    other.request_name("org.example.Unrelated").await.unwrap();
    assert!(
        quiet(&mut signals).await,
        "another name's owner change reached the watch"
    );
}

/// The watch's answers kept in a [`super::HostCache`]: one change report per coming and going, and the stored answer
/// follows the host without any read opening the bus.
#[tokio::test]
async fn the_host_cache_reports_each_change_once_and_reads_open_no_connection() {
    let Some(bus) = private_bus() else {
        eprintln!("NOT RUN: tray host cache test needs the dbus-daemon program");
        assert!(
            std::env::var("VW_REQUIRE_ALL").as_deref() != Ok("1"),
            "dbus-daemon is required under VW_REQUIRE_ALL=1"
        );
        return;
    };
    let watcher = zbus::connection::Builder::address(bus.address.as_str())
        .unwrap()
        .name("org.kde.StatusNotifierWatcher")
        .unwrap()
        .serve_at("/StatusNotifierWatcher", Watcher { host: true })
        .unwrap()
        .build()
        .await
        .unwrap();
    let cache = Arc::new(super::HostCache::default());
    assert!(!cache.get(), "the cache starts with no host");
    let opened = Arc::new(AtomicUsize::new(0));
    let answers = Arc::new(AtomicUsize::new(0));
    let (send, mut changes) = mpsc::unbounded_channel();
    let watch = {
        let (opened, address) = (opened.clone(), bus.address.clone());
        let (cache, answers) = (cache.clone(), answers.clone());
        tokio::spawn(async move {
            super::watch_host_on(
                move || {
                    opened.fetch_add(1, Ordering::SeqCst);
                    let address = address.clone();
                    async move { connect(&address).await }
                },
                move |host| {
                    answers.fetch_add(1, Ordering::SeqCst);
                    if cache.update(host) {
                        let _ = send.send(host);
                    }
                },
            )
            .await;
        })
    };
    let within = Duration::from_secs(3);
    assert_eq!(
        tokio::time::timeout(within, changes.recv()).await.unwrap(),
        Some(true)
    );
    // Several one-second checks with the host present: no further change report, and reads are answered at once.
    tokio::time::sleep(Duration::from_millis(2_500)).await;
    for _ in 0..1000 {
        assert!(cache.get());
    }
    assert!(
        changes.try_recv().is_err(),
        "a check without a change was reported"
    );
    assert!(
        answers.load(Ordering::SeqCst) >= 3,
        "the watch stopped checking"
    );
    let iface = watcher
        .object_server()
        .interface::<_, Watcher>("/StatusNotifierWatcher")
        .await
        .unwrap();
    iface.get_mut().await.host = false;
    Watcher::status_notifier_host_unregistered(iface.signal_emitter())
        .await
        .unwrap();
    assert_eq!(
        tokio::time::timeout(within, changes.recv()).await.unwrap(),
        Some(false)
    );
    assert!(!cache.get());
    assert_eq!(opened.load(Ordering::SeqCst), 1, "a read opened the bus");
    watch.abort();
}
