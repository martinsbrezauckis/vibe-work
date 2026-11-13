//! Server-free fixture for feature boundary tests; owns a scratch profile and refuses every native prompt.
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use vw_gate::{Confirmation, ConfirmationRequest, Confirmer};
static NEXT: AtomicUsize = AtomicUsize::new(0);
struct NoPrompt;
impl Confirmer for NoPrompt {
    async fn confirm(&self, _: &ConfirmationRequest) -> Confirmation {
        panic!("unexpected prompt");
    }
}
pub(crate) struct Fixture {
    pub core: crate::Core,
    pub root: PathBuf,
}
impl Fixture {
    pub async fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "vw-feature-core-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let home = root.join("app-data/vibe");
        std::fs::create_dir_all(&home).unwrap();
        let root = vw_platform::canonical_plain(&root).unwrap();
        let home = root.join("app-data/vibe");
        let config = crate::CoreConfig {
            start_vibe: false,
            agent_tools: Some(crate::AgentToolsConfig::new(
                root.join("no-tools-spawned"),
                root.join("app-data/tools"),
                None,
            )),
            instructions: crate::InstructionsConfig {
                cli_home: Some(root.join("cli")),
            },
            ..crate::CoreConfig::default()
        };
        let core = Box::pin(crate::Core::start(
            vw_supervisor::SupervisorConfig::new(root.join("no-vibe-spawned"), home, root.clone()),
            Arc::new(vw_supervisor::NoSecret),
            NoPrompt,
            config,
        ))
        .await
        .unwrap();
        Self { core, root }
    }
    pub fn load(&self, id: &str, cwd: &Path, extra: &[PathBuf], active: bool) {
        std::fs::create_dir_all(cwd).unwrap();
        let _ = self.core.shared.store.load(serde_json::json!({"session":{"id":id,"cwd":cwd,"workspaceRoots":extra,"status":{"type":if active {"running"}else{"idle"}}},"entries":[],"turns":[]}),vw_session::LoadKind::New).unwrap();
    }
    pub async fn finish(self) {
        self.core.shutdown().await;
        std::fs::remove_dir_all(self.root).unwrap();
    }
}
