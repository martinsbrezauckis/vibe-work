//! Real pinned supervisor fixtures with a loopback model and side-file observations of the child environment.
//! Used by lifecycle and launch tests; protocol replies always come from the Vibe process.
use serde_json::{Value, json};
use std::path::PathBuf;
use vw_supervisor::SupervisorConfig;
pub struct Rig {
    pub config: SupervisorConfig,
    pub dir: PathBuf,
    pub plan: PathBuf,
}
pub async fn rig(name: &str, plan: Value) -> Rig {
    let dir = super::processes::scratch(name);
    let model =
        super::scripted_model::ScriptedModel::start(super::shared_real_vibe::text_script()).await;
    let mut config = super::shared_real_vibe::supervisor_config(&dir, &model);
    super::core::test_keyring::disable(&mut config);
    super::core::record(&mut config, &dir);
    let plan = super::process_fault::install(&mut config, &dir, plan);
    Rig { config, dir, plan }
}
pub async fn plain(name: &str) -> Rig {
    rig(name, json!({})).await
}
