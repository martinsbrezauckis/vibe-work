//! Wire-contract checks for host tools and additive runtime fields; uses only generated types.
use serde_json::{Value, json};

fn round_trip<T: serde::de::DeserializeOwned + serde::Serialize>(
    raw: Value,
) -> serde_json::Result<()> {
    let expected = raw.clone();
    let decoded: T = serde_json::from_value(raw)?;
    assert_eq!(serde_json::to_value(decoded)?, expected);
    Ok(())
}

#[test]
fn tool_input_is_required_even_when_it_is_null() -> serde_json::Result<()> {
    let mut raw = json!({"callId":"c", "name":"t", "input":null, "toolCallId":"t",
        "caller":{"sessionId":"s", "rootSessionId":"s"}});
    round_trip::<vw_protocol::ClientToolCallParams>(raw.clone())?;
    raw.as_object_mut().unwrap().remove("input");
    assert!(serde_json::from_value::<vw_protocol::ClientToolCallParams>(raw).is_err());
    Ok(())
}

#[test]
fn output_schema_is_required_even_when_it_is_null() -> serde_json::Result<()> {
    let mut raw = json!({"name":"t", "namespace":"n", "description":"", "inputSchema":{}, "outputSchema":null});
    round_trip::<vw_protocol::ClientToolDefinitionParams>(raw.clone())?;
    raw.as_object_mut().unwrap().remove("outputSchema");
    assert!(serde_json::from_value::<vw_protocol::ClientToolDefinitionParams>(raw).is_err());
    Ok(())
}

#[test]
fn skill_winner_budget_reason_and_cli_identity_round_trip() -> serde_json::Result<()> {
    for active in [true, false] {
        round_trip::<vw_protocol::SkillSummary>(
            json!({"name":"s", "description":"", "prompt":"", "active":active}),
        )?;
    }
    let turn = json!({"id":"t", "sessionId":"s", "status":"interrupted", "startedAt":0});
    round_trip::<vw_protocol::PublicTurn>(turn.clone())?;
    for reason in ["token_limit", "price_limit"] {
        let mut budget = turn.clone();
        budget["budgetStopReason"] = json!(reason);
        round_trip::<vw_protocol::PublicTurn>(budget)?;
    }
    round_trip::<vw_protocol::PublicError>(
        json!({"message":"Missing price", "code":"pricing_unavailable"}),
    )?;
    round_trip::<vw_protocol::ProviderCredentialSourceKind>(json!("cli_oidc"))
}
