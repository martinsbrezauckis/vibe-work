//! Bakes the tool catalog's build identity into the core. Cargo reruns on workspace Rust source changes or an explicit build id.
//! Release builders may supply `VW_BUILD_ID` for reproducibility; local rebuilds otherwise get a fresh timestamp.
use std::{
    env,
    time::{SystemTime, UNIX_EPOCH},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-env-changed=VW_BUILD_ID");
    // Tool definitions also live in sibling crates; rebuilding those must retire the catalog cache too.
    println!("cargo:rerun-if-changed=..");
    println!("cargo:rerun-if-changed=../../Cargo.toml");
    println!("cargo:rerun-if-changed=../../Cargo.lock");
    let identity = match env::var("VW_BUILD_ID") {
        Ok(value) if !value.trim().is_empty() => value,
        _ => format!(
            "build-{}",
            SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
        ),
    };
    println!("cargo:rustc-env=VW_TOOL_BUILD_ID={identity}");
    Ok(())
}
