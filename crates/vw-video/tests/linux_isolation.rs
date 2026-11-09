//! Positive and negative Linux sandbox acceptance through the production launcher and supervisor.
#![cfg(all(target_os = "linux", feature = "helper"))]
use serde_json::json;

#[tokio::test]
#[ignore = "requires working nested Linux namespaces and seccomp"]
async fn snapshot_is_readable_but_network_files_writes_and_children_are_denied() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("input");
    let outside = root.path().join("private-canary");
    std::fs::write(&source, b"allowed").unwrap();
    std::fs::write(&outside, b"private").unwrap();
    let source = source.canonicalize().unwrap();
    let input =
        vw_video::input::Input::copy(std::fs::File::open(&source).unwrap(), &source).unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let spec = json!({"outside":outside,"address":listener.local_addr().unwrap().to_string()});
    let job = vw_video::protocol::Job {
        id: spec.to_string(),
        request: serde_json::from_value(json!({"path":source,"selection":{"kind":"timestamps","times_ms":[0]},"output":"frame"})).unwrap(),
    };
    let helper = std::path::PathBuf::from(
        std::env::var_os("VW_VIDEO_PROBE").expect("native isolation probe"),
    );
    let result = vw_video::sandbox::extract(&helper, input, job)
        .await
        .err()
        .unwrap();
    let probes: serde_json::Value = serde_json::from_str(&result).unwrap();
    for name in [
        "read_allowed",
        "read_denied",
        "write_denied",
        "network_denied",
        "socket_denied",
        "process_denied",
        "no_rights",
        "bounded",
    ] {
        assert_eq!(probes[name], true, "{name}: {probes}");
    }
    println!("ISOLATION | {probes}");
}
