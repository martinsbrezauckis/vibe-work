//! Replays Vibe error replies and malformed/channel responses. The client maps
//! codes, preserves reasons/details, and delegates input validation to the server.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use serde_json::{Value, json};
use vw_skills::{
    ProtocolError, ProtocolErrorCode, RuntimeMutationStatus, SkillsClient, SkillsConvertResponse,
    SkillsError, SkillsImportParams, SkillsListParams, SkillsTransport, TransportError,
};

struct Rejection {
    method: String,
    params: Value,
    error: ProtocolError,
}

impl SkillsTransport for Rejection {
    fn request(
        &self,
        method: &str,
        params: Value,
    ) -> impl std::future::Future<Output = Result<Value, TransportError>> + Send {
        assert_eq!(method, self.method);
        assert_eq!(params, self.params);
        std::future::ready(Err(TransportError::Reply(self.error.clone())))
    }
}

#[tokio::test]
async fn maps_recorded_error_codes_and_preserves_plain_reasons() {
    let errors: Vec<Value> = serde_json::from_str(include_str!("fixtures/errors.json")).unwrap();
    for fixture in errors {
        let reply: ProtocolError = serde_json::from_value(fixture["error"].clone()).unwrap();
        let method = fixture["method"].as_str().unwrap();
        let params = match method {
            "skills/import" => {
                json!({"sessionId":"s","skillId":"id","version":null,"alias":null,"scope":"global"})
            }
            "skills/remove" => json!({"sessionId":"s","name":"n","scope":"global"}),
            "skills/setEnabled" => json!({"sessionId":"s","name":"review","enabled":false}),
            "skills/list" => json!({"sessionId":"s"}),
            _ => panic!("Uncovered fixture method"),
        };
        let client = SkillsClient::new(Rejection {
            method: method.to_owned(),
            params: params.clone(),
            error: reply.clone(),
        });
        let error = match method {
            "skills/import" => client
                .import(&serde_json::from_value(params).unwrap())
                .await
                .unwrap_err(),
            "skills/remove" => client
                .remove(&serde_json::from_value(params).unwrap())
                .await
                .unwrap_err(),
            "skills/setEnabled" => client
                .set_enabled(&serde_json::from_value(params).unwrap())
                .await
                .unwrap_err(),
            "skills/list" => client
                .list(&serde_json::from_value(params).unwrap())
                .await
                .unwrap_err(),
            _ => panic!("Uncovered fixture method"),
        };
        assert_eq!(error.to_string(), reply.message);
        let ((ProtocolErrorCode::InvalidParams, SkillsError::InvalidInput(original))
        | (ProtocolErrorCode::Conflict, SkillsError::Busy(original))
        | (ProtocolErrorCode::Forbidden, SkillsError::AccessDenied(original))
        | (ProtocolErrorCode::MethodNotFound, SkillsError::Unsupported(original))
        | (ProtocolErrorCode::InternalError, SkillsError::Server(original))) = (reply.code, error)
        else {
            panic!("Wrong error classification")
        };
        assert_eq!(original, reply);
    }
}

#[tokio::test]
async fn passes_through_import_strings_and_version_scope_without_validation() {
    for skill_id in [
        "/tmp/skill-folder",
        "https://example.invalid/skill",
        "registry-id",
    ] {
        let params = SkillsImportParams {
            session_id: "s".into(),
            skill_id: skill_id.into(),
            version: Some(-1),
            alias: Some("unusual alias".into()),
            scope: "other-scope".into(),
        };
        let reply = ProtocolError {
            code: ProtocolErrorCode::InvalidParams,
            message: "invalid skill scope: 'other-scope'".into(),
            data: json!({"retained":true}),
        };
        let client = SkillsClient::new(Rejection {
            method: "skills/import".into(),
            params: serde_json::to_value(&params).unwrap(),
            error: reply.clone(),
        });
        let SkillsError::InvalidInput(received) = client.import(&params).await.unwrap_err() else {
            panic!("Expected Vibe rejection")
        };
        assert_eq!(received, reply);
    }
}

struct Broken(bool);

impl SkillsTransport for Broken {
    fn request(
        &self,
        _method: &str,
        _params: Value,
    ) -> impl std::future::Future<Output = Result<Value, TransportError>> + Send {
        std::future::ready(if self.0 {
            Ok(json!({"skills":"wrong type"}))
        } else {
            Err(TransportError::Unavailable("Vibe is disconnected".into()))
        })
    }
}

#[tokio::test]
async fn distinguishes_wire_and_channel_failures() {
    let params = SkillsListParams {
        session_id: "s".into(),
    };
    assert!(matches!(
        SkillsClient::new(Broken(true))
            .list(&params)
            .await
            .unwrap_err(),
        SkillsError::Wire(_)
    ));
    let disconnected = SkillsClient::new(Broken(false))
        .list(&params)
        .await
        .unwrap_err();
    assert!(matches!(disconnected, SkillsError::Unavailable(_)));
    assert_eq!(disconnected.to_string(), "Vibe is disconnected");
}

#[test]
fn pending_runtime_status_and_conversion_noop_are_preserved() {
    let mut fixture: Value =
        serde_json::from_str(include_str!("fixtures/convertLocal.json")).unwrap();
    fixture["result"]["status"] = json!("pending");
    fixture["result"]["converted"] = json!(false);
    let response: SkillsConvertResponse =
        serde_json::from_value(fixture["result"].clone()).unwrap();
    assert_eq!(response.status, RuntimeMutationStatus::Pending);
    assert!(!response.converted);
    assert_eq!(serde_json::to_value(response).unwrap(), fixture["result"]);
}
