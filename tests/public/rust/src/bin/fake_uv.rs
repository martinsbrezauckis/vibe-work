//! A stand-in for Astral's `uv` in the Vibe install tests (the fixture): it answers the three commands the installer
//! runs (`python install`, `venv`, `pip install`) by building the layout the real ones leave (a Python folder,
//! a venv with `mistral_vibe-<version>.dist-info` and a `vibe-app-server` executable copied from the fake Vibe
//! server), and records every call so a test can check what the installer passed.
//!
//! Environment (the installer passes these through its test-only `extra_env`):
//! - `FAKE_UV_LOG`: JSON lines `{argv, env: {UV_*/PIP_* names}, pid, cwd}` are appended here;
//! - `FAKE_UV_SERVER`: the executable copied in as `vibe-app-server`;
//! - `FAKE_UV_VIBE_VERSION`: the dist-info version written, unless the `-r` file pins `mistral-vibe==<v>`
//!   (an update fixture can install another version than the rig's);
//! - `FAKE_UV_MODE`: `ok` (default), `fail-pip` (a build error, exit 1), `network-pip` (uv's DNS error words,
//!   exit 2), `slow-pip` (starts a child that sleeps, records its pid, then sleeps 120 s itself: for cancel and
//!   timeout tests, which then check both processes are gone).
//!
//! `fake_uv sleep-child` is that child.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

fn log(entry: &serde_json::Value) {
    if let Ok(path) = std::env::var("FAKE_UV_LOG")
        && let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
    {
        // One write per line: `writeln!` on an unbuffered file formats in pieces (several write calls), and the step
        // and its child log at the same moment (v7), so pieces of two lines interleaved. O_APPEND keeps one write whole.
        let mut line = entry.to_string();
        line.push('\n');
        let _ = file.write_all(line.as_bytes());
    }
}

fn bin_dir(venv: &Path) -> PathBuf {
    venv.join(if cfg!(windows) { "Scripts" } else { "bin" })
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let env: serde_json::Map<String, serde_json::Value> = std::env::vars()
        .filter(|(k, _)| k.starts_with("UV_") || k.starts_with("PIP_"))
        .map(|(k, v)| (k, serde_json::Value::String(v)))
        .collect();
    log(&serde_json::json!({
        "argv": args, "env": env, "pid": std::process::id(),
        "cwd": std::env::current_dir().map(|p| p.display().to_string()).unwrap_or_default(),
    }));
    let mode = std::env::var("FAKE_UV_MODE").unwrap_or_else(|_| "ok".to_owned());
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    match words.as_slice() {
        ["sleep-child"] => std::thread::sleep(Duration::from_secs(120)),
        ["python", "install", version, ..] => {
            let dir = PathBuf::from(std::env::var("UV_PYTHON_INSTALL_DIR").unwrap_or_default())
                .join("cpython-fake");
            std::fs::create_dir_all(&dir).expect("python dir");
            println!("Installed Python {version} (fake) in {}", dir.display());
        }
        ["venv", .., dir] => {
            std::fs::create_dir_all(bin_dir(Path::new(dir))).expect("venv dir");
            println!("Creating virtual environment at: {dir}");
        }
        ["pip", "install", "--python", python, rest @ ..] => {
            pip(Path::new(python), &mode, requirement_version(rest));
        }
        other => {
            eprintln!("fake uv: unexpected command {other:?}");
            std::process::exit(64);
        }
    }
}

/// The `mistral-vibe==<version>` pinned by the `-r` file (the fixture: an update installs another version than the
/// rig's `FAKE_UV_VIBE_VERSION`), if any.
fn requirement_version(rest: &[&str]) -> Option<String> {
    let file = rest.windows(2).find(|w| w[0] == "-r").map(|w| w[1])?;
    std::fs::read_to_string(file)
        .ok()?
        .lines()
        .find_map(|l| l.trim().strip_prefix("mistral-vibe==").map(str::to_owned))
        .and_then(|rest| rest.split_whitespace().next().map(str::to_owned))
}

fn pip(python: &Path, mode: &str, pinned: Option<String>) {
    println!("Resolved 101 packages in 12ms");
    match mode {
        "fail-pip" => {
            eprintln!("error: fake build failure in package `example`");
            std::process::exit(1);
        }
        "network-pip" => {
            eprintln!("error: Failed to fetch: `https://pypi.org/simple/mistral-vibe/`");
            eprintln!("  Caused by: dns error: failed to lookup address information");
            std::process::exit(2);
        }
        "slow-pip" => {
            let exe = std::env::current_exe().expect("own exe");
            let mut child = std::process::Command::new(exe)
                .arg("sleep-child")
                .spawn()
                .expect("child");
            log(&serde_json::json!({"child": child.id()}));
            println!("Downloading packages (slow, fake)");
            std::thread::sleep(Duration::from_secs(120));
            // Reached only when nobody killed the tree (the tests always do).
            let _ = child.wait();
            return;
        }
        _ => {}
    }
    let venv = python
        .parent()
        .and_then(Path::parent)
        .expect("venv of the python path");
    let version = pinned.unwrap_or_else(|| {
        std::env::var("FAKE_UV_VIBE_VERSION").unwrap_or_else(|_| "0.0.0".to_owned())
    });
    let site = if cfg!(windows) {
        venv.join("Lib").join("site-packages")
    } else {
        venv.join("lib").join("python3.12").join("site-packages")
    };
    std::fs::create_dir_all(site.join(format!("mistral_vibe-{version}.dist-info")))
        .expect("dist-info");
    let server = bin_dir(venv).join(if cfg!(windows) {
        "vibe-app-server.exe"
    } else {
        "vibe-app-server"
    });
    std::fs::copy(
        std::env::var("FAKE_UV_SERVER").expect("FAKE_UV_SERVER"),
        &server,
    )
    .expect("server copy");
    println!("Installed 101 packages in 3ms");
}
