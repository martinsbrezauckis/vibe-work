//! Agent profile listings project live choices without changing the helper's saved pairing records.
//! Uses the isolated core and fake status transport; a closed profile returns under the same ID.
use super::gateway;
use serde_json::json;

#[tokio::test]
async fn inactive_profiles_are_not_choices_and_their_pairings_survive_reconnect() {
    let profiles = json!([
        {"profileId":"chrome:healthy","connected":true,"extensionVersion":"0.1.6"},
        {"profileId":"chrome:deleted","connected":false},
        {"profileId":"chrome:closed","connected":false}
    ]);
    let core = super::edge_agent_tests::fixture("browser-inactive-choices", profiles.clone()).await;
    let shared = &core.shared;
    let record = shared
        .chrome_control
        .dir()
        .unwrap()
        .join("paired-profiles.json");
    let original = b"fixture pairing record: deleted and merely closed";
    std::fs::write(&record, original).unwrap();
    let reply = gateway::call(shared, "s", "chrome_status", json!({}))
        .await
        .unwrap();
    let listed = &reply["vwMcpResult"]["structuredContent"]["untrustedPageContent"]["profiles"];
    assert_eq!(listed.as_array().unwrap().len(), 1, "{reply}");
    assert_eq!(listed[0]["profileId"], "chrome:healthy");
    for inactive in ["chrome:deleted", "chrome:closed"] {
        assert!(!reply.to_string().contains(inactive), "{reply}");
    }
    assert_eq!(std::fs::read(&record).unwrap(), original);
    let mut args = json!({"profileId":"chrome"});
    super::profile_ids::resolve_browser(&mut args, &json!({"profiles":profiles})).unwrap();
    assert_eq!(args["profileId"], "chrome:healthy");
    // Closing a browser is never an unpair operation. Its ID returns on the next live status.
    shared.chrome_control.owner.answering(|_, _| {
        json!({"structuredContent":{"profiles":[
            {"profileId":"chrome:closed","connected":true,"extensionVersion":"0.1.6"}
        ]}})
    });
    let reply = gateway::call(shared, "s", "chrome_status", json!({}))
        .await
        .unwrap();
    assert_eq!(
        reply["vwMcpResult"]["structuredContent"]["untrustedPageContent"]["profiles"][0]["profileId"],
        "chrome:closed"
    );
    assert_eq!(std::fs::read(&record).unwrap(), original);
    core.shutdown().await;
}
