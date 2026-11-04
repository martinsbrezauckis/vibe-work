//! Fixed sandbox launcher. System libraries/plugins are read-only; home, credentials and network are absent.
//! Unsupported or denied isolation is a refusal, never an unrestricted decoder fallback.
use crate::{
    input::Input,
    protocol::{Job, Reply},
};
#[cfg(target_os = "linux")]
use process_wrap::tokio::CommandWrap;
use std::path::Path;
#[cfg(target_os = "linux")]
use std::time::Duration;
/// Starts a fresh restricted process; dropping this future cancels and reaps the job.
/// # Errors
/// Isolation unavailable, resource/output limit or decoding failure.
#[allow(
    clippy::unused_async,
    reason = "same asynchronous contract on unsupported native launchers"
)]
pub async fn extract(helper: &Path, input: Input, job: Job) -> Result<Reply, String> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (helper, input, job);
        Err("backend_unavailable: native helper sandbox is not deployed".into())
    }
    #[cfg(target_os = "linux")]
    {
        let mut job = job;
        job.request.path = "/input".into();
        let command = command(helper, &input)?;
        let bytes = serde_json::to_vec(&job).map_err(|_| "decode_failed: request serialization")?;
        let reply = crate::process::capture(command, Duration::from_secs(35), bytes).await?;
        if let Ok(error) = serde_json::from_slice::<serde_json::Value>(&reply)
            && let Some(message) = error["error"].as_str()
        {
            return Err(message.chars().take(256).collect());
        }
        let reply: Reply =
            serde_json::from_slice(&reply).map_err(|_| "decode_failed: helper reply shape")?;
        crate::protocol::validate(&job, &reply)?;
        Ok(reply)
    }
}
#[cfg(target_os = "linux")]
fn command(helper: &Path, input: &Input) -> Result<CommandWrap, String> {
    let helper = helper
        .canonicalize()
        .map_err(|_| "backend_unavailable: video helper is missing")?;
    Ok(CommandWrap::with_new("/usr/bin/bwrap", |c| {
        c.env_clear()
            .env("PATH", "/usr/bin")
            .env("LANG", "C")
            .env("GST_PLUGIN_PATH_1_0", "")
            .env("GST_PLUGIN_SYSTEM_PATH_1_0", "/plugins")
            .env("GST_REGISTRY_1_0", "/nonexistent/registry")
            .env("GST_REGISTRY_FORK", "no")
            .env("GST_PLUGIN_SCANNER_1_0", "/nonexistent/scanner")
            .env("GIO_USE_VFS", "local")
            .env("ORC_CODE", "backup")
            .env("MALLOC_ARENA_MAX", "1");
        c.args([
            "--unshare-all",
            "--die-with-parent",
            "--cap-drop",
            "ALL",
            "--ro-bind",
            "/usr",
            "/usr",
        ]);
        for path in ["/lib", "/lib64"] {
            if Path::new(path).exists() {
                c.args(["--ro-bind", path, path]);
            }
        }
        c.args([
            "--proc", "/proc", "--dev", "/dev", "--tmpfs", "/tmp", "--dir", "/etc",
        ]);
        if Path::new("/etc/ld.so.cache").exists() {
            c.args(["--ro-bind", "/etc/ld.so.cache", "/etc/ld.so.cache"]);
        }
        c.args(["--dir", "/plugins"]);
        for path in system_plugins() {
            if let Some(name) = path.file_name() {
                c.arg("--ro-bind")
                    .arg(&path)
                    .arg(Path::new("/plugins").join(name));
            }
        }
        c.arg("--ro-bind")
            .arg(input.path())
            .arg("/input")
            .arg("--ro-bind")
            .arg(helper)
            .arg("/worker")
            .args(["--chdir", "/", "--", "/worker"]);
    }))
}

/// Installed plugin files used by the explicit pipeline. Never scans user paths or unrelated device plugins.
#[cfg(target_os = "linux")]
#[must_use]
pub fn system_plugins() -> Vec<std::path::PathBuf> {
    let mut paths = Vec::new();
    for name in [
        "coreelements",
        "app",
        "typefindfunctions",
        "isomp4",
        "matroska",
        "videoconvertscale",
        "videoconvert",
        "videoscale",
        "videoparsersbad",
        "libav",
        "vpx",
        "openh264",
        "de265",
        "aom",
        "dav1d",
    ] {
        for root in [
            "/usr/lib/x86_64-linux-gnu/gstreamer-1.0",
            "/usr/lib64/gstreamer-1.0",
            "/usr/lib/gstreamer-1.0",
        ] {
            let path = Path::new(root).join(format!("libgst{name}.so"));
            if path.is_file() {
                paths.push(path);
                break;
            }
        }
    }
    paths
}
