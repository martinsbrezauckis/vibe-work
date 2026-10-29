//! The public/private test rigs share one pinned-runtime selector, rejecting a real older environment.
//! Uses the runtime's metadata interpreter only; no sessions, model calls or credentials.
use std::path::PathBuf;

#[test]
fn default_runtime_lives_in_the_versioned_environment() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let server = vw_public_tests::runtime::default_server(&root);
    assert!(
        server.starts_with(root.join("data/vibe-3.0.3/venv")),
        "{}",
        server.display()
    );
}

#[test]
fn selected_runtime_matches_the_pin_and_reports_python() {
    let runtime = vw_public_tests::runtime::selected().unwrap();
    assert_eq!(runtime.vibe_version, "3.0.3");
    assert!(
        runtime.python_version.starts_with("3.14."),
        "{}",
        runtime.python_version
    );
}
