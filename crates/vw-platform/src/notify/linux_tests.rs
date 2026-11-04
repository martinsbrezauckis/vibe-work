//! Linux notification wire regression on a private D-Bus server advertising markup and hyperlinks.
//! Calls the real `DesktopNotifier`; no notification reaches the person's desktop.
use super::{DesktopNotifier, NotificationRequest, Notifier};
use std::{collections::HashMap, sync::mpsc, time::Duration};

const TITLE: &str = "<a href=\"https://example.invalid/\">Open approval</a> & text";
const ESCAPED: &str =
    "&lt;a href=\"https://example.invalid/\"&gt;Open approval&lt;/a&gt; &amp; text";

struct Server(mpsc::Sender<(String, String, Vec<String>)>);

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl Server {
    #[allow(clippy::unused_self)] // D-Bus interface methods require a receiver.
    fn get_capabilities(&self) -> Vec<&str> {
        vec!["body", "body-markup", "body-hyperlinks", "actions"]
    }

    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: &str,
        replaces_id: u32,
        app_icon: &str,
        summary: &str,
        body: &str,
        actions: Vec<String>,
        hints: HashMap<String, zbus::zvariant::OwnedValue>,
        expire_timeout: i32,
    ) -> u32 {
        let _ = (app_name, replaces_id, app_icon, hints, expire_timeout);
        assert!(self.0.send((summary.into(), body.into(), actions)).is_ok());
        1
    }
}

#[test]
fn linux_notification_emits_plain_text_on_markup_servers() {
    if std::env::var_os("VW_NOTIFY_PRIVATE_BUS").is_none() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("bus.conf");
        // This disposable bus must also run inside test boxes without access to host AppArmor policy.
        std::fs::write(&config, r#"<busconfig>
<type>session</type><listen>unix:tmpdir=/tmp</listen><auth>EXTERNAL</auth>
<apparmor mode="disabled"/>
<policy context="default"><allow own="*"/><allow send_destination="*"/><allow receive_sender="*"/></policy>
</busconfig>"#).unwrap();
        let output = std::process::Command::new("dbus-run-session")
            .arg("--config-file")
            .arg(config)
            .arg("--")
            .arg(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "notify::linux_tests::linux_notification_emits_plain_text_on_markup_servers",
                "--nocapture",
            ])
            .env("VW_NOTIFY_PRIVATE_BUS", "1")
            .env("XDG_RUNTIME_DIR", dir.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let (sent, received) = mpsc::channel();
    let _server = zbus::blocking::connection::Builder::session()
        .unwrap()
        .name("org.freedesktop.Notifications")
        .unwrap()
        .serve_at("/org/freedesktop/Notifications", Server(sent))
        .unwrap()
        .build()
        .unwrap();
    let capabilities = notify_rust::get_capabilities().unwrap();
    assert!(capabilities.iter().any(|c| c == "body-markup"));
    assert!(capabilities.iter().any(|c| c == "body-hyperlinks"));
    DesktopNotifier::new("test", false)
        .show(
            NotificationRequest {
                title: TITLE.into(),
                body: TITLE.into(),
            },
            Box::new(|| {}),
        )
        .unwrap();
    let (summary, body, actions) = received.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(body, ESCAPED);
    // The summary is plain text by the Freedesktop spec, so it is sent unchanged.
    assert_eq!(summary, TITLE);
    assert_eq!(actions, ["default", "Open"]);
}
