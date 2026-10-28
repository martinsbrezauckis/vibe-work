//! A core without Vibe for unit tests of core state (Stop generations, asks, receipts): no server is started, and
//! a native prompt fails the test. Used by: `stops_tests`, `chrome_control::tests`.
use std::sync::Arc;

use vw_gate::{Confirmation, ConfirmationRequest, Confirmer};

struct NoPrompt;
impl Confirmer for NoPrompt {
    fn confirm(
        &self,
        _: &ConfirmationRequest,
    ) -> impl std::future::Future<Output = Confirmation> + Send {
        std::future::poll_fn(|_| -> std::task::Poll<Confirmation> {
            panic!("this test must not open a native prompt")
        })
    }
}

/// A started core with no Vibe server, its folders under the workspace's `target/tmp/<name>-<pid>`.
pub(crate) async fn start(name: &str) -> crate::Core {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../../target/tmp/{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let config = crate::CoreConfig {
        start_vibe: false,
        ..crate::CoreConfig::default()
    };
    crate::Core::start(
        vw_supervisor::SupervisorConfig::new(
            dir.join("no-server-requested"),
            dir.join("vibe-home"),
            dir.clone(),
        ),
        Arc::new(vw_supervisor::NoSecret),
        NoPrompt,
        config,
    )
    .await
    .unwrap()
}
