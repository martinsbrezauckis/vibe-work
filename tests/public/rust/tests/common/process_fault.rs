//! Real-process fault plans and side-file observations; depends on the fault launcher, not synthetic RPC methods.
//! Called by supervisor and storage-error tests; every protocol response still comes from real Vibe.
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use vw_supervisor::SupervisorConfig;
pub fn install(config: &mut SupervisorConfig, dir: &Path, mut plan: Value) -> PathBuf {
    if let Some(pair) = config
        .launcher
        .windows(2)
        .find(|pair| pair[0] == "--launch")
    {
        plan["childLaunchFile"] = json!(pair[1].to_string_lossy());
    }
    if plan["onRequest"]
        .as_array()
        .is_some_and(|rules| rules.iter().any(|rule| rule["readDelayMs"].is_number()))
    {
        let python = vw_supervisor::server_python(&super::shared_real_vibe::server_bin())
            .expect("the pinned Vibe's Python");
        config
            .extra_env
            .push(("VW_FAULT_PYTHON".to_owned(), python.into_os_string()));
    }
    let file = dir.join("process-fault.json");
    std::fs::write(&file, plan.to_string()).unwrap();
    let mut args = vec![
        PathBuf::from(env!("CARGO_BIN_EXE_real_process_fault")).into_os_string(),
        "--plan".into(),
        file.clone().into_os_string(),
        "--".into(),
    ];
    args.append(&mut config.launcher);
    config.launcher = args;
    file
}
pub fn info(plan: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(plan.with_extension("info")).unwrap()).unwrap()
}
pub fn crash(plan: &Path, code: i32, stderr: &str) {
    // Stop the server first, so nothing sent after this call is answered before the kill lands.
    #[cfg(windows)]
    {
        let pid = u32::try_from(info(plan)["server"].as_u64().unwrap()).unwrap();
        assert!(super::launchers::suspend(pid, true), "could not stop {pid}");
    }
    #[cfg(unix)]
    {
        let pid = info(plan)["server"].as_u64().unwrap().to_string();
        assert!(
            std::process::Command::new("kill")
                .args(["-STOP", &pid])
                .status()
                .unwrap()
                .success()
        );
    }
    std::fs::write(
        plan.with_extension("control"),
        json!({"exit":code,"stderr":stderr}).to_string(),
    )
    .unwrap();
}
