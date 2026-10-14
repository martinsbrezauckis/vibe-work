//! Helper process for supervisor tests that need a *separate* parent: it runs a `Supervisor`
//! over the fake server and prints one JSON line about it.
//!
//! - `supervise_probe env <fake>`: reads its own real environment (as the app would), spawns the
//!   fake, prints `{"env": {...}}` (the child's environment), shuts down.
//! - `supervise_probe tree <fake> [extra fake flags...]`: spawns the fake with a grandchild (plus any
//!   extra flags, e.g. `--spawn-agent --ignore-eof`), prints `{"leader": pid, "grandchild": pid,
//!   "agent": pid|null}`, then sleeps until killed (the test kills this process hard and checks that the
//!   server tree dies with it).

use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use vw_supervisor::{NoSecret, Supervisor, SupervisorConfig};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args[1].clone();
    let fake = std::path::PathBuf::from(&args[2]);
    let dir = std::env::temp_dir().join(format!("vw-probe-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut config = SupervisorConfig::new(fake, dir.join("vibe-home"), dir.clone());
    if mode == "tree" {
        config.extra_args = vec!["--spawn-grandchild".into()];
        config
            .extra_args
            .extend(args.iter().skip(3).map(std::ffi::OsString::from));
    }
    let supervisor = Supervisor::start(config, Arc::new(NoSecret)).await.unwrap();
    let conn = supervisor.connection(Duration::from_secs(5)).await.unwrap();
    if mode == "env" {
        let env = conn.request("env/dump", json!({})).await.unwrap();
        println!("{env}");
        supervisor.shutdown().await;
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }
    let info = conn.request("pid/info", json!({})).await.unwrap();
    println!(
        "{}",
        json!({"leader": info["pid"], "grandchild": info["grandchild"], "agent": info["agent"]})
    );
    tokio::time::sleep(Duration::from_secs(300)).await;
}
