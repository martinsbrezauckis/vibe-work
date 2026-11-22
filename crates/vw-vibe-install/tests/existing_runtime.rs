//! Opt-in native version/refusal regression against a real old Vibe in a disposable probe home.
//! Depends on the install crate and `VW_TEST_OLD_VIBE_SERVER`; never downloads or touches the person's Vibe home.
use std::path::PathBuf;
use vw_vibe_install::{InstallerConfig, Limits, Sources, VibeInstaller, layout};

#[tokio::test]
#[ignore = "set VW_TEST_OLD_VIBE_SERVER to a genuine Vibe 2.26.1 app server"]
async fn an_old_personal_vibe_names_its_version_and_stays_refused() {
    let server = PathBuf::from(std::env::var_os("VW_TEST_OLD_VIBE_SERVER").unwrap());
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("vw-old-personal-{}", std::process::id()));
    assert!(!root.exists(), "use a fresh disposable root");
    let current = Sources::pinned(&vw_vibe_install::this_target())
        .unwrap()
        .vibe_version;
    let own = layout::Layout::new(root.clone(), &current).unwrap();
    own.save_choice(&layout::Choice {
        path: server,
        version: "2.26.1".into(),
    })
    .unwrap();
    let personal = VibeInstaller::new(InstallerConfig {
        root: root.clone(),
        sources: Sources::pinned(&vw_vibe_install::this_target()),
        parent_env: std::env::vars_os().collect(),
        extra_env: vec![],
        discovery: vw_supervisor::Discovery {
            explicit: None,
            path_var: None,
            home: None,
        },
        limits: Limits::default(),
    });
    assert!(personal.server().is_none());
    personal.check_saved_existing().await;
    let view = personal.view();
    let found = &view.existing[0];
    assert_eq!(found.version.as_deref(), Some("2.26.1"));
    assert!(!found.supported);
    assert_eq!(
        found.warning.as_deref(),
        Some(vw_vibe_install::existing::version_warning("2.26.1", &current).as_str())
    );
    assert!(personal.use_existing(&found.path).await.is_err());
    assert!(personal.server().is_none());
    assert!(!personal.upgrade_previous_private().unwrap());
    std::fs::remove_dir_all(root).unwrap();
}
