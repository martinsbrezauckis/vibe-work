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
    return tokio::time::timeout(Duration::from_millis(500), probe())
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or(false);
    #[cfg(not(target_os = "linux"))]
    true
}

#[cfg(target_os = "linux")]
async fn probe() -> zbus::Result<bool> {
    let bus = zbus::Connection::session().await?;
    let dbus = zbus::fdo::DBusProxy::new(&bus).await?;
    if !dbus
        .name_has_owner(zbus::names::BusName::try_from(
            "org.kde.StatusNotifierWatcher",
        )?)
        .await?
    {
        return Ok(false);
    }
    let watcher = zbus::Proxy::new(
        &bus,
        "org.kde.StatusNotifierWatcher",
        "/StatusNotifierWatcher",
        "org.kde.StatusNotifierWatcher",
    )
    .await?;
    watcher
        .get_property::<bool>("IsStatusNotifierHostRegistered")
        .await
}

/// Monitors owner and host-loss signals; the periodic check also catches a broken bus or a
/// watcher that changes its property without emitting the required signal. Never creates a tray.
#[cfg(target_os = "linux")]
pub async fn watch_host(on_change: impl Fn(bool) + Send) {
    use futures_util::StreamExt;
    loop {
        if let Ok(bus) = zbus::Connection::session().await {
            let rule = zbus::MatchRule::builder()
                .msg_type(zbus::message::Type::Signal)
                .build();
            if let Ok(mut signals) = zbus::MessageStream::for_match_rule(rule, &bus, Some(16)).await
            {
                let mut timer = tokio::time::interval(Duration::from_secs(1));
                loop {
                    tokio::select! {
                        signal = signals.next() => {
                            let Some(Ok(signal)) = signal else { break; };
                            let header = signal.header();
                            let relevant = match header.member().map(zbus::names::MemberName::as_str) {
                                Some("NameOwnerChanged") => signal.body().deserialize::<(String, String, String)>()
                                    .is_ok_and(|(name, _, _)| name == "org.kde.StatusNotifierWatcher"),
                                Some("StatusNotifierHostUnregistered") => header.interface()
                                    .is_some_and(|name| name.as_str() == "org.kde.StatusNotifierWatcher"),
                                _ => false,
                            };
                            if !relevant { continue; }
                        }
                        _ = timer.tick() => {}
                    }
                    on_change(host_available().await);
                }
            }
        }
        on_change(false);
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}
