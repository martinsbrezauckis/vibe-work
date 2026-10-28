//! The scripted model as its own process, for tests that start the real app or the real Vibe themselves
//! (desktop and browser runners): `scripted_model --script FILE [--fallback URL]`. It prints
//! one JSON line `{"base": "http://127.0.0.1:<port>/v1", "server": "http://127.0.0.1:<port>"}` once it listens, logs
//! every request it gets as JSON lines to `--log FILE` (path, stream, tool names, the last user text, the step), and runs
//! until its stdin closes. The model itself is `tests/common/scripted_model.rs` (its header has the wire and the script
//! format); `--fallback` passes every non-chat path on (a test's own Mistral stand-in for key checks, speech, OCR).
//!
//! Depends on: `scripted_model` (via `#[path]`), tokio. Called by: native and browser test launchers.

#[allow(dead_code)]
#[path = "../../tests/common/fake_mistral.rs"]
mod fake_mistral;
#[path = "../../tests/common/scripted_model.rs"]
mod scripted_model;
#[path = "../../tests/common/scripted_platform.rs"]
mod scripted_platform;
#[path = "../../tests/common/scripted_registry.rs"]
mod scripted_registry;

use std::io::Write as _;
use std::path::PathBuf;

use tokio::io::AsyncReadExt;

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let Some(script) = arg(&args, "--script") else {
        eprintln!("usage: scripted_model --script FILE [--fallback URL] [--log FILE]");
        std::process::exit(2);
    };
    // Opt-in native document tests share the strict HTTP OCR/key-test fixture.
    let fixture = if args.iter().any(|arg| arg == "--mistral-fixture") {
        Some(fake_mistral::start().await)
    } else {
        None
    };
    let model = scripted_model::ScriptedModel::start_with_fallback(
        scripted_model::Script::from_file(&PathBuf::from(script)),
        arg(&args, "--fallback").or_else(|| fixture.as_ref().map(|f| f.base())),
    )
    .await;
    println!(
        "{}",
        serde_json::json!({"base": model.base(), "server": model.server()})
    );
    let _ = std::io::stdout().flush();
    if let Some(log) = arg(&args, "--log") {
        model.log_to(PathBuf::from(log));
    }
    let mut sink = Vec::new();
    let _ = tokio::io::stdin().read_to_end(&mut sink).await;
    // Let the request log catch up before the process ends.
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
}
