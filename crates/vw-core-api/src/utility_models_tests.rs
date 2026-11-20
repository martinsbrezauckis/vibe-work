//! Which model the core's own Mistral calls fall back to ([`super`]).
//!
//! PREDICT (written before the change): the commit draft used a fixed `mistral-large-4` and the Libraries/image calls
//! failed outright when Mistral refused `mistral-medium-latest`; nothing read the chat's active model, so these
//! helpers do not exist and the module does not compile without the change.
//! - the active model is Vibe's provider `name` of `runtime.config.activeModel` (never the alias);
//! - without a chat, the Settings default is `defaultModelAlias` looked up in the runtime's model list;
//! - only a model refusal (400/422 invalid model, 404 unknown model) leads to a second try; a key, plan, billing or
//!   rate problem is not retried with another model.

use serde_json::json;
use vw_keys::WorkFailure;
use vw_mistral::work::WorkError;

use super::{active_in, default_in, model_refused};

fn runtime() -> serde_json::Value {
    json!({"config": {
        "activeModel": {"name": "mistral-large-4-0", "alias": "large-4", "displayName": "Mistral Large 4"},
        "defaultModelAlias": "mistral-medium-3.5",
        "models": [
            {"name": "mistral-vibe-cli-latest", "alias": "mistral-medium-3.5"},
            {"name": "mistral-large-4-0", "alias": "large-4"}
        ]
    }})
}

#[test]
fn the_active_model_is_the_provider_name_vibe_reports() {
    assert_eq!(active_in(&runtime()).as_deref(), Some("mistral-large-4-0"));
    let blank = json!({"config": {"activeModel": {"name": "  ", "alias": "x"}}});
    assert_eq!(active_in(&blank), None, "an empty name is no model");
    assert_eq!(active_in(&json!({})), None);
}

#[test]
fn the_settings_default_is_found_in_the_runtimes_model_list() {
    assert_eq!(
        default_in(&runtime()).as_deref(),
        Some("mistral-vibe-cli-latest")
    );
    let missing =
        json!({"config": {"defaultModelAlias": "gone", "models": [{"name": "a", "alias": "b"}]}});
    assert_eq!(
        default_in(&missing),
        None,
        "an unlisted default is not guessed"
    );
}

#[test]
fn only_a_model_refusal_is_retried_with_another_model() {
    for refused in [
        WorkError::Rejected(400),
        WorkError::Rejected(422),
        WorkError::NotFound,
    ] {
        assert!(model_refused(&WorkFailure::Work(refused)));
    }
    for other in [
        WorkError::Unauthorized,
        WorkError::Payment,
        WorkError::Forbidden,
        WorkError::RateLimited,
        WorkError::TooLarge,
    ] {
        assert!(!model_refused(&WorkFailure::Work(other)));
    }
    assert!(!model_refused(&WorkFailure::KeyNeeded(
        vw_keyproxy::KeyNeed::Unlock
    )));
}

#[tokio::test]
async fn without_any_chat_runtime_there_is_no_model_to_fall_back_to() {
    let fixture = crate::feature_test_core::Fixture::new().await;
    assert_eq!(super::active_model(&fixture.core.shared, None).await, None);
    assert_eq!(
        super::retry_model(&fixture.core.shared, None, "mistral-medium-latest").await,
        None
    );
}
