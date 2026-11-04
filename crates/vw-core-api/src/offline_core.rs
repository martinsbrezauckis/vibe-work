//! A core without Vibe for unit tests of core state (Stop generations, asks, receipts): no server is started, and
//! a native prompt fails the test. Used by: `stops_tests`, `chrome_control::tests`.
use std::{fmt::Write as _, sync::Arc};

use vw_gate::{Confirmation, ConfirmationRequest, Confirmer};

struct NoPrompt;
impl Confirmer for NoPrompt {
    fn confirm(
        &self,
        _: &ConfirmationRequest,
    ) -> impl std::future::Future<Output = Confirmation> + Send {
        std::future::poll_fn(|_| -> std::task::Poll<Confirmation> {
            panic!("this test must not open a native prompt")
        })
    }
}

/// A started core with no Vibe server, isolated under `VW_TEST_ROOT` (or the workspace test folder).
pub(crate) async fn start(name: &str) -> crate::Core {
    start_with(name, |_, _| {}).await
}

fn fixture_folder_name(name: &str) -> String {
    // Escape into a lowercase portable alphabet so IDs cannot collide after sanitizing or case folding.
    let mut folder = String::with_capacity(name.len());
    for byte in name.bytes() {
        match byte {
            b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' => folder.push(char::from(byte)),
            _ => write!(folder, "~{byte:02x}").unwrap(),
        }
    }
    write!(folder, "-{}", std::process::id()).unwrap();
    folder
}

/// [`start`] with a config change (for example a Chrome Control package path, which nothing reads until Set up).
pub(crate) async fn start_with(
    name: &str,
    edit: impl FnOnce(&mut crate::CoreConfig, &std::path::Path),
) -> crate::Core {
    let root = std::env::var_os("VW_TEST_ROOT").map_or_else(
        || std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp"),
        std::path::PathBuf::from,
    );
    let name = fixture_folder_name(name);
    let dir = root.join(&name);
    std::fs::create_dir_all(&dir).unwrap();
    let dir = vw_platform::canonical_plain(&dir).unwrap();
    let mut config = crate::CoreConfig {
        start_vibe: false,
        ..crate::CoreConfig::default()
    };
    edit(&mut config, &dir);
    // Chrome fixtures never discover or launch the person's browser.
    if config.chrome_control.user_home.is_none() {
        config.chrome_control.user_home = Some(dir.join("chrome-home"));
    }
    if config.chrome_control.launcher.is_none() {
        config.chrome_control.launcher = Some(Arc::new(|_| Ok(())));
    }
    // ... nor touch Chrome's real per-user registry keys (Windows): a test that keeps Chrome's default root gets a
    // disposable key of its own under the shared test parent.
    if config.chrome_control.windows_registry_root
        == crate::ChromeControlConfig::default().windows_registry_root
    {
        config.chrome_control.windows_registry_root =
            format!(r"HKCU\Software\VibeWorkTests\core-{name}\NativeMessagingHosts");
    }
    // Keep the startup state off every caller's future stack, including the larger Windows configuration.
    Box::pin(crate::Core::start(
        vw_supervisor::SupervisorConfig::new(
            dir.join("no-server-requested"),
            dir.join("vibe-home"),
            dir.clone(),
        ),
        Arc::new(vw_supervisor::NoSecret),
        NoPrompt,
        config,
    ))
    .await
    .unwrap()
}

#[test]
fn fixture_folder_names_are_safe_on_windows() {
    for name in [
        "person-stop-refusal-chrome:p",
        "browser-agent-stop-edge:p",
        "<>:\"/\\|?*",
        "\0\n\u{1f}",
        "profile. ",
        ".",
        "..",
        "CON",
        "",
    ] {
        let folder = fixture_folder_name(name);
        assert!(
            !folder
                .chars()
                .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c)),
            "invalid Windows fixture folder: {folder:?}"
        );
        assert!(!folder.ends_with(['.', ' ']));
        assert_eq!(std::path::Path::new(&folder).components().count(), 1);
    }
}

#[test]
fn fixture_folder_names_keep_distinct_ids_separate() {
    let names = [
        "chrome:p",
        "chrome_p",
        "chrome~3ap",
        "chrome/p",
        "chrome\\p",
        "Chrome:p",
    ];
    let folders: std::collections::BTreeSet<_> = names
        .iter()
        .map(|name| fixture_folder_name(name).to_ascii_lowercase())
        .collect();
    assert_eq!(folders.len(), names.len());
}
