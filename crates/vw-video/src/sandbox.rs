//! Fixed sandbox launcher. System libraries/plugins are read-only; home, credentials and network are absent.
//! Unsupported or denied isolation is a refusal, never an unrestricted decoder fallback.
use crate::{
    input::Input,
    protocol::{Job, Reply},
};
use process_wrap::tokio::CommandWrap;
use std::path::Path;
/// Starts a fresh restricted process; dropping this future cancels and reaps the job.
/// # Errors
/// Isolation unavailable, resource/output limit or decoding failure.
pub async fn extract(helper: &Path, input: Input, job: Job) -> Result<Reply, String> {
    let mut job = job;
    #[cfg(target_os = "linux")]
    {
        job.request.path = "/input".into();
    }
    #[cfg(target_os = "macos")]
    {
        job.request.path = input.path().to_string_lossy().into_owned();
    }
    #[cfg(windows)]
    let stage = stage_windows(helper, &input)?;
    #[cfg(windows)]
    {
        job.request.path = stage.path().join("input").to_string_lossy().into_owned();
    }
    #[cfg(target_os = "linux")]
    let command = command(helper, &input)?;
    #[cfg(target_os = "macos")]
    let command = native_command(helper, "--worker", Path::new(&job.request.path));
    #[cfg(windows)]
    let command = native_command(
        &stage.path().join("vw-video-helper.exe"),
        "--sandbox-launch",
        Path::new(&job.request.path),
    );
    let bytes = serde_json::to_vec(&job).map_err(|_| "decode_failed: request serialization")?;
    let reply =
        crate::process::capture(command, vw_platform::video_sandbox::DEADLINE, bytes).await?;
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

#[cfg(not(target_os = "linux"))]
fn native_command(helper: &Path, mode: &str, input: &Path) -> CommandWrap {
    CommandWrap::with_new(helper, |c| {
        c.env_clear().arg(mode).arg(input);
        #[cfg(windows)]
        {
            c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW for the console helper bootstrap.
            // The bootstrap registers its AppContainer profile, which needs LOCALAPPDATA; the worker never gets it.
            for name in ["SystemRoot", "LOCALAPPDATA"] {
                if let Some(value) = std::env::var_os(name) {
                    c.env(name, value);
                }
            }
        }
        #[cfg(unix)]
        c.current_dir("/");
    })
}

#[cfg(windows)]
fn stage_windows(helper: &Path, input: &Input) -> Result<tempfile::TempDir, String> {
    let stage = tempfile::tempdir().map_err(|_| "backend_unavailable: media worker directory")?;
    std::fs::copy(helper, stage.path().join("vw-video-helper.exe"))
        .map_err(|_| "backend_unavailable: video helper is missing")?;
    std::fs::copy(input.path(), stage.path().join("input"))
        .map_err(|_| "backend_unavailable: sandbox input snapshot")?;
    Ok(stage)
}
#[cfg(target_os = "linux")]
fn command(helper: &Path, input: &Input) -> Result<CommandWrap, String> {
    let helper = helper
        .canonicalize()
        .map_err(|_| "backend_unavailable: video helper is missing")?;
    let packaged = helper.with_file_name("vw-video-sandbox");
    let launcher = if packaged.is_file() {
        packaged.as_path()
    } else {
        Path::new("/usr/bin/bwrap")
    };
    let libraries = helper
        .parent()
        .and_then(Path::parent)
        .map(|root| root.join("lib"))
        .filter(|dir| dir.join("libgstreamer-1.0.so.0").is_file());
    Ok(CommandWrap::with_new(launcher, |c| {
        c.env_clear()
            .env("PATH", "/usr/bin")
            .env("LANG", "C")
            .env("GST_PLUGIN_PATH_1_0", "")
            .env("GST_PLUGIN_SYSTEM_PATH_1_0", "/plugins")
            .env("GST_REGISTRY_1_0", "/proc/self/registry")
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
        if let Some(lib) = &libraries {
            c.arg("--ro-bind").arg(lib).arg("/runtime/lib").args([
                "--setenv",
                "LD_LIBRARY_PATH",
                "/runtime/lib",
            ]);
        }
        for mut path in system_plugins() {
            // Bundled support plugins match the bundled GStreamer ABI; codecs remain the computer's own.
            if let Some(lib) = &libraries
                && let Some(name) = path.file_name()
            {
                let bundled = lib.join("gstreamer-1.0").join(name);
                if bundled.is_file() {
                    path = bundled;
                }
            }
            if let Some(name) = path.file_name() {
                c.arg("--ro-bind")
                    .arg(&path)
                    .arg(Path::new("/plugins").join(name));
            }
        }
        // The host may lack a container/conversion plugin which the AppImage supplies. The video parsers
        // (videoparsersbad) are not bundled: they come from the computer's plugins-bad, like the decoders.
        if let Some(lib) = &libraries {
            for name in [
                "coreelements",
                "app",
                "typefindfunctions",
                "isomp4",
                "matroska",
                "videoconvertscale",
            ] {
                let filename = format!("libgst{name}.so");
                let path = lib.join("gstreamer-1.0").join(&filename);
                if path.is_file() {
                    c.arg("--ro-bind")
                        .arg(path)
                        .arg(Path::new("/plugins").join(filename));
                }
            }
        }
        c.arg("--ro-bind")
            .arg(input.path())
            .arg("/input")
            .arg("--ro-bind")
            .arg(helper)
            .arg("/worker")
            .args(["--remount-ro", "/", "--chdir", "/", "--", "/worker"]);
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

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    #[test]
    fn packaged_launcher_and_libraries_are_selected() {
        let root = tempfile::tempdir().unwrap();
        let bin = root.path().join("usr/bin");
        let lib = root.path().join("usr/lib");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&lib).unwrap();
        for file in [
            bin.join("vw-video-helper"),
            bin.join("vw-video-sandbox"),
            lib.join("libgstreamer-1.0.so.0"),
        ] {
            std::fs::write(file, b"fixture").unwrap();
        }
        let path = bin.join("vw-video-helper").canonicalize().unwrap();
        let input = Input::copy(std::fs::File::open(&path).unwrap(), &path).unwrap();
        let mut command = command(&path, &input).unwrap();
        let command = command.command_mut().as_std();
        assert_eq!(command.get_program(), bin.join("vw-video-sandbox"));
        let args: Vec<_> = command.get_args().collect();
        assert!(args.contains(&lib.as_os_str()));
        assert!(args.contains(&std::ffi::OsStr::new("/runtime/lib")));
    }
}
