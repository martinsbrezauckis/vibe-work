//! Separate-parent supervisor probe over pinned real Vibe and a loopback scripted model.
//! `env SERVER` records the child environment; `tree SERVER [--ignore-eof] [--spawn-agent]` reports actual PIDs.
mod probe_support;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use vw_supervisor::{NoSecret, Supervisor};
#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dir = std::env::temp_dir().join(format!("vw-probe-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut faults: Vec<_> = args.iter().skip(3).cloned().collect();
    if args[1] == "tree" {
        faults.push("--spawn-grandchild".into());
    }
    let (config, plan) = probe_support::config(args[2].clone().into(), &dir, &faults).await;
    let supervisor = Supervisor::start(config, Arc::new(NoSecret)).await.unwrap();
    supervisor
        .current()
        .unwrap()
        .request("session/list", json!({"limit":1}))
        .await
        .unwrap();
    let info = probe_support::info(&plan);
    if args[1] == "env" {
        println!("{}", json!({"env":info["env"]}));
        supervisor.shutdown().await;
        std::fs::remove_dir_all(&dir).unwrap();
    } else {
        println!(
            "{}",
            json!({"leader":info["pid"],"grandchild":info["grandchild"],"agent":info["agent"]})
        );
        tokio::time::sleep(Duration::from_secs(300)).await;
    }
}
