//! Shared real-server startup for separate-parent crash probes; owns only disposable files and a local model.
//! The fault launcher records environment/PIDs and forwards real Vibe bytes without synthetic methods.
#![allow(dead_code)]
#[path = "../../../tests/common/real_vibe.rs"]
mod real_vibe;
#[path = "../../../tests/common/scripted_model.rs"]
mod scripted_model;
#[path = "../../../tests/common/scripted_platform.rs"]
mod scripted_platform;
#[path = "../../../tests/common/scripted_registry.rs"]
mod scripted_registry;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
pub async fn config(
    server: PathBuf,
    dir: &Path,
    args: &[String],
) -> (vw_supervisor::SupervisorConfig, PathBuf) {
    let model = scripted_model::ScriptedModel::start(real_vibe::text_script()).await;
    let mut config = real_vibe::supervisor_config(dir, &model);
    config.program = server;
    let file = dir.join("fault.json");
    std::fs::write(&file,json!({"spawnGrandchild":args.iter().any(|a|a=="--spawn-grandchild"),"spawnAgent":args.iter().any(|a|a=="--spawn-agent"),"ignoreEof":args.iter().any(|a|a=="--ignore-eof")}).to_string()).unwrap();
    let bin = std::env::current_exe()
        .unwrap()
        .with_file_name(if cfg!(windows) {
            "real_process_fault.exe"
        } else {
            "real_process_fault"
        });
    config.launcher = vec![
        bin.into_os_string(),
        "--plan".into(),
        file.clone().into_os_string(),
        "--".into(),
    ];
    (config, file)
}
pub fn info(file: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(file.with_extension("info")).unwrap()).unwrap()
}
