//! Connection failures stay errors; depends on the real bus adapter, run without a display or bus.
#![cfg(target_os = "linux")]

use vw_desktop::Desktop;

#[tokio::test]
async fn malformed_and_missing_bus_addresses_are_errors() {
    assert!(Desktop::connect_at("not a D-Bus address").await.is_err());
    let missing = concat!(
        "unix:path=",
        env!("CARGO_MANIFEST_DIR"),
        "/tests/no-bus/socket"
    );
    assert!(Desktop::connect_at(missing).await.is_err());
}
