//! Builds the small Linux page-process extension against the same `WebKit` ABI as the host.
//! Cargo embeds the result; no engine download or new build dependency is involved.
//! On Windows (MSVC) it embeds the Common Controls 6 dependency into this package's own test programs: the
//! `vw-platform` calls they link (`TaskDialogIndirect`, `SetWindowSubclass`) exist only in v6, which the app selects
//! through its own Tauri manifest. `rustc-link-arg` reaches only this package's linked targets, never the app.
use std::{env, path::PathBuf, process::Command};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=native/frame_extension.c");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        // Test programs have no app manifest; without Common Controls 6 they cannot start (0xC0000139).
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg=/MANIFESTDEPENDENCY:type='win32' name='Microsoft.Windows.Common-Controls' \
             version='6.0.0.0' processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'"
        );
    }
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("linux") {
        return Ok(());
    }
    let flags = Command::new("pkg-config")
        .args(["--cflags", "--libs", "webkit2gtk-web-extension-4.1"])
        .output()?;
    assert!(flags.status.success(), "WebKit extension pkg-config failed");
    let output = PathBuf::from(env::var_os("OUT_DIR").ok_or("Cargo output directory missing")?)
        .join("libagent_frames.so");
    let status = Command::new(env::var_os("CC").unwrap_or_else(|| "cc".into()))
        .args([
            "-O2",
            "-shared",
            "-fPIC",
            "-Wall",
            "-Wextra",
            "-Werror",
            "native/frame_extension.c",
            "-o",
        ])
        .arg(output)
        .args(String::from_utf8(flags.stdout)?.split_whitespace())
        .status()?;
    assert!(status.success(), "WebKit frame extension build failed");
    Ok(())
}
