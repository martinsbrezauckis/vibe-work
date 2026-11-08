//! Checks the optional context read against the generated Vibe response before it reaches the UI.
//! Called by `dispatch_call`; never changes the session or contributes input to the agent.
use serde_json::Value;
use vw_protocol::{ContextCategoryId, SessionContextReadResponse};

use crate::error::{CoreError, ErrorCode};

pub(crate) fn validate(result: &Value) -> Result<(), CoreError> {
    let parsed: SessionContextReadResponse =
        serde_json::from_value(result.clone()).map_err(|_| invalid())?;
    let breakdown = parsed.breakdown;
    if [
        breakdown.context_window,
        breakdown.compaction_threshold,
        breakdown.used_tokens,
        breakdown.total_characters,
    ]
    .iter()
    .any(|count| *count < 0)
    {
        return Err(invalid());
    }
    let categories = breakdown.categories.unwrap_or_default();
    let mut sum = 0_i64;
    for (index, category) in categories.iter().enumerate() {
        if !matches!(
            category.id,
            ContextCategoryId::System
                | ContextCategoryId::Tools
                | ContextCategoryId::Skills
                | ContextCategoryId::Conversation
        ) || category.characters < 0
            || category.estimated_tokens < 0
            || categories[..index]
                .iter()
                .any(|other| other.id == category.id)
        {
            return Err(invalid());
        }
        sum = sum
            .checked_add(category.estimated_tokens)
            .ok_or_else(invalid)?;
    }
    if !categories.is_empty() && sum != breakdown.used_tokens {
        return Err(invalid());
    }
    Ok(())
}

fn invalid() -> CoreError {
    CoreError::new(
        ErrorCode::ServerError,
        "Vibe returned an invalid context breakdown.",
    )
    .with_method("session/context/read")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn malformed_or_inconsistent_readings_never_reach_the_window() {
        let valid = json!({"breakdown": {
            "contextWindow":1000, "compactionThreshold":800, "usedTokens":100,
            "measured":true, "totalCharacters":400,
            "categories":[{"id":"conversation", "characters":400, "estimatedTokens":100}]
        }});
        assert!(validate(&valid).is_ok());
        for (pointer, bad) in [
            ("/breakdown/measured", json!(null)),
            ("/breakdown/contextWindow", json!(-1)),
            ("/breakdown/usedTokens", json!(101)),
            ("/breakdown/categories/0/id", json!("files")),
            ("/breakdown/categories/0/characters", json!(-1)),
        ] {
            let mut raw = valid.clone();
            *raw.pointer_mut(pointer).unwrap() = bad;
            assert!(validate(&raw).is_err(), "{pointer}");
        }
        let mut missing = valid;
        missing["breakdown"]["categories"] = json!([]);
        assert!(
            validate(&missing).is_ok(),
            "a saved total can have no split"
        );
    }
}
