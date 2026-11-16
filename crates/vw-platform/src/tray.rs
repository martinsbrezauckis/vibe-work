//! `StatusNotifier` availability on Linux, bounded to 500 ms. Depends on zbus;
//! desktop setup and every close consult this, and a signal monitor restores an orphaned window.
#[cfg(target_os = "linux")]
use std::time::Duration;

/// A reachable watcher with a registered host. XEmbed-only trays deliberately count as absent.
/// Other desktop platforms provide the native tray themselves.
// One async interface across platforms; only Linux needs to await the desktop bus.
#[cfg_attr(not(target_os = "linux"), allow(clippy::unused_async))]
pub async fn host_available() -> bool {
    #[cfg(target_os = "linux")]
    return bounded(async { probe(&zbus::Connection::session().await?).await }).await;
    #[cfg(not(target_os = "linux"))]
    true
}

/// The tray host as the host watch ([`watch_host`]) last reported it, for readers that must not open the bus
/// themselves (the Settings row asks on demand; every close still probes). Starts with no host.
#[derive(Debug, Default)]
pub struct HostCache(std::sync::atomic::AtomicBool);

impl HostCache {
    /// The last stored answer.
    #[must_use]
    pub fn get(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::Acquire)
    }

    /// Stores `available`; true when it differs from the answer before (the host came or went).
    pub fn update(&self, available: bool) -> bool {
        self.0.swap(available, std::sync::atomic::Ordering::AcqRel) != available
    }
}

/// A probe's answer within 500 ms; a slow, broken or absent bus counts as no host.
#[cfg(target_os = "linux")]
async fn bounded(probe: impl std::future::Future<Output = zbus::Result<bool>>) -> bool {
    tokio::time::timeout(Duration::from_millis(500), probe)
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or(false)
}

#[cfg(target_os = "linux")]
async fn probe(bus: &zbus::Connection) -> zbus::Result<bool> {
    let dbus = zbus::fdo::DBusProxy::new(bus).await?;
    if !dbus
        .name_has_owner(zbus::names::BusName::try_from(WATCHER)?)
        .await?
    {
        return Ok(false);
    }
    let watcher = zbus::Proxy::new(bus, WATCHER, "/StatusNotifierWatcher", WATCHER).await?;
    watcher
        .get_property::<bool>("IsStatusNotifierHostRegistered")
        .await
}

#[cfg(target_os = "linux")]
const WATCHER: &str = "org.kde.StatusNotifierWatcher";

/// Monitors owner and host-loss signals; the periodic check also catches a broken bus or a
/// watcher that changes its property without emitting the required signal. Never creates a tray.
#[cfg(target_os = "linux")]
pub async fn watch_host(on_change: impl Fn(bool) + Send) {
    watch_host_on(zbus::Connection::session, on_change).await;
}

/// [`watch_host`] over the connections `connect` opens: one per bus session, which carries both the signal
/// subscription and every probe; a new one only after the bus ends the subscription.
#[cfg(target_os = "linux")]
async fn watch_host_on<C, F>(connect: C, on_change: impl Fn(bool) + Send)
where
    C: Fn() -> F,
    F: std::future::Future<Output = zbus::Result<zbus::Connection>>,
{
    use futures_util::StreamExt;
    loop {
        if let Ok(bus) = connect().await
            && let Ok(mut signals) = watcher_signals(&bus).await
        {
            let mut timer = tokio::time::interval(Duration::from_secs(1));
            loop {
                tokio::select! {
                    signal = signals.next() => {
                        let Some(Ok(_)) = signal else { break; };
                    }
                    _ = timer.tick() => {}
                }
                on_change(bounded(probe(&bus)).await);
            }
        }
        on_change(false);
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

/// Only the two signals that change the answer: the watcher name changing owner, and the watcher losing its host.
#[cfg(target_os = "linux")]
async fn watcher_signals(
    bus: &zbus::Connection,
) -> zbus::Result<impl futures_util::Stream<Item = zbus::Result<zbus::Message>> + Unpin> {
    let owner = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender("org.freedesktop.DBus")?
        .interface("org.freedesktop.DBus")?
        .member("NameOwnerChanged")?
        .arg(0, WATCHER)?
        .build();
    let host = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface(WATCHER)?
        .member("StatusNotifierHostUnregistered")?
        .build();
    let owner = zbus::MessageStream::for_match_rule(owner, bus, Some(16)).await?;
    let host = zbus::MessageStream::for_match_rule(host, bus, Some(16)).await?;
    Ok(futures_util::stream::select(owner, host))
}

#[cfg(all(test, target_os = "linux"))]
#[path = "tray_tests.rs"]
mod tests;

#[cfg(all(target_os = "linux", feature = "desktop-tray"))]
pub mod indicator;
