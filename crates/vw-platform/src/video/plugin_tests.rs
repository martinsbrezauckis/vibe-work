//! Actual registry omission tests. Every child sees only a private subset of installed system plugins.
use super::Decoder;
use std::{path::Path, process::Command};
#[test]
#[ignore = "requires the verified corpus and native system GStreamer plugins"]
fn omitted_plugins_name_the_missing_system_support() {
    let system = Path::new("/usr/lib/x86_64-linux-gnu/gstreamer-1.0");
    let plugins = [
        ("coreelements", "filesrc"),
        ("app", "appsink"),
        ("matroska", "matroskademux"),
        ("vpx", "video/x-vp9"),
    ];
    for (omit, expected) in plugins {
        let dir = tempfile::tempdir().unwrap();
        for (plugin, _) in plugins {
            if plugin != omit {
                let name = format!("libgst{plugin}.so");
                std::os::unix::fs::symlink(system.join(&name), dir.path().join(name)).unwrap();
            }
        }
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "video::plugin_tests::plugin_child",
                "--ignored",
                "--nocapture",
            ])
            .env("GST_PLUGIN_SYSTEM_PATH_1_0", dir.path())
            .env("GST_PLUGIN_PATH_1_0", "")
            .env("GST_REGISTRY_1_0", dir.path().join("registry"))
            .env("GST_REGISTRY_FORK", "no")
            .env("VW_VIDEO_MISSING_ELEMENT", expected)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "omitted {omit}: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
#[test]
#[ignore = "registry-isolated child; invoked by the omission test"]
fn plugin_child() {
    let Ok(expected) = std::env::var("VW_VIDEO_MISSING_ELEMENT") else {
        return;
    };
    let root = std::env::var_os("VW_VIDEO_CORPUS").expect("owned corpus");
    let result = Decoder::open(&Path::new(&root).join("vp9-cues.webm"), 1280);
    let Err(reason) = result else {
        panic!("omitted plugin still decoded")
    };
    assert!(reason.contains(&expected), "{reason}, expected {expected}");
    assert!(
        reason.starts_with("backend_unavailable") || reason.starts_with("unsupported_codec"),
        "{reason}"
    );
}
