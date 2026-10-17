//! Checks serde against the pinned Pydantic schemas and model-written fixtures.
//! Default omission and additive fields are tested through the public models.

#![allow(clippy::unwrap_used)]

use std::path::Path;

use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use vw_skills::{
    McpSourceSummary, ModelConfigView, RegistryRefView, RuntimeMutationResponse, SkillScope,
    SkillSource, SkillSummary, SkillsCatalogParams, SkillsCatalogResponse,
    SkillsConvertLocalParams, SkillsConvertResponse, SkillsDetailParams, SkillsDetailResponse,
    SkillsImportParams, SkillsInstalledParams, SkillsInstalledResponse, SkillsListParams,
    SkillsListResponse, SkillsRemoveParams, SkillsSetAliasParams, SkillsSetEnabledParams,
    SkillsSetLatestParams, SkillsSetVersionParams, SkillsUpdatesParams, SkillsUpdatesResponse,
    SkillsVersionsParams, SkillsVersionsResponse, ThinkingLevel,
};

fn schema(name: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/schema")
        .join(format!("{name}.json"));
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn omit_defaults_and_add_fields(value: &mut Value, model: &Value) {
    if let Some(reference) = model["$ref"].as_str() {
        omit_defaults_and_add_fields(value, &schema(reference.strip_prefix("#/$defs/").unwrap()));
    } else if let Some(union) = model["anyOf"].as_array() {
        if !value.is_null() {
            let variant = union.iter().find(|item| item["type"] != "null").unwrap();
            omit_defaults_and_add_fields(value, variant);
        }
    } else if let Some(properties) = model["properties"].as_object() {
        let object = value.as_object_mut().unwrap();
        let required = model["required"].as_array();
        for (field, description) in properties {
            let optional = !required.is_some_and(|fields| fields.contains(&json!(field)));
            let omit = object.get(field).is_some_and(|current| {
                optional
                    && (description.get("default") == Some(current)
                        || (description.get("default").is_none()
                            && (current == &json!([]) || current == &json!({}))))
            });
            if omit {
                object.remove(field);
            } else if let Some(current) = object.get_mut(field) {
                omit_defaults_and_add_fields(current, description);
            }
        }
        object.insert("futureField".into(), json!({"additive": true}));
    } else if model["type"] == "array" {
        for item in value.as_array_mut().unwrap() {
            omit_defaults_and_add_fields(item, &model["items"]);
        }
    }
}

fn check<T: Serialize + DeserializeOwned>(name: &str, original: &Value) {
    let mut extended = original.clone();
    omit_defaults_and_add_fields(&mut extended, &schema(name));
    let parsed: T = serde_json::from_value(extended).unwrap();
    assert_eq!(
        serde_json::to_value(parsed).unwrap(),
        *original,
        "Schema defaults for {name}"
    );
}

macro_rules! contract_test {
    ($name:ident, $file:literal, $params:ty, $response:ty) => {
        #[test]
        fn $name() {
            let fixture: Value = serde_json::from_str(include_str!($file)).unwrap();
            check::<$params>(stringify!($params), &fixture["params"]);
            check::<$response>(stringify!($response), &fixture["result"]);
        }
    };
}

contract_test!(
    list,
    "fixtures/list.json",
    SkillsListParams,
    SkillsListResponse
);
contract_test!(
    installed,
    "fixtures/installed.json",
    SkillsInstalledParams,
    SkillsInstalledResponse
);
contract_test!(
    catalog,
    "fixtures/catalog.json",
    SkillsCatalogParams,
    SkillsCatalogResponse
);
contract_test!(
    versions,
    "fixtures/versions.json",
    SkillsVersionsParams,
    SkillsVersionsResponse
);
contract_test!(
    updates,
    "fixtures/updates.json",
    SkillsUpdatesParams,
    SkillsUpdatesResponse
);
contract_test!(
    detail,
    "fixtures/detail.json",
    SkillsDetailParams,
    SkillsDetailResponse
);
contract_test!(
    import,
    "fixtures/import.json",
    SkillsImportParams,
    RuntimeMutationResponse
);
contract_test!(
    set_version,
    "fixtures/setVersion.json",
    SkillsSetVersionParams,
    RuntimeMutationResponse
);
contract_test!(
    set_latest,
    "fixtures/setLatest.json",
    SkillsSetLatestParams,
    RuntimeMutationResponse
);
contract_test!(
    set_alias,
    "fixtures/setAlias.json",
    SkillsSetAliasParams,
    RuntimeMutationResponse
);
contract_test!(
    remove,
    "fixtures/remove.json",
    SkillsRemoveParams,
    RuntimeMutationResponse
);
contract_test!(
    set_enabled,
    "fixtures/setEnabled.json",
    SkillsSetEnabledParams,
    RuntimeMutationResponse
);
contract_test!(
    convert_local,
    "fixtures/convertLocal.json",
    SkillsConvertLocalParams,
    SkillsConvertResponse
);

#[test]
fn optional_fields_are_none_and_declared_defaults_match_vibe() {
    let import: SkillsImportParams =
        serde_json::from_value(json!({"sessionId":"s","skillId":"id"})).unwrap();
    assert_eq!(import.version, None);
    assert_eq!(import.alias, None);
    assert_eq!(import.scope, "global");
    let detail: SkillsDetailResponse = serde_json::from_value(json!({})).unwrap();
    assert_eq!(detail.detail, None);
    assert_eq!(detail.body, None);
    let ref_view: RegistryRefView =
        serde_json::from_value(json!({"skillId":"id","version":1})).unwrap();
    assert_eq!(ref_view.alias, None);
    let skill: SkillSummary =
        serde_json::from_value(json!({"name":"n","description":"d","prompt":"p"})).unwrap();
    assert_eq!(skill.registry, None);
    assert_eq!(skill.source, SkillSource::Local);
    assert_eq!(skill.scope, SkillScope::Global);
    assert!(skill.enabled && skill.user_invocable && !skill.locked);
    let mcp: McpSourceSummary = serde_json::from_value(
        json!({"name":"server","kind":"server","transport":"stdio","status":"connected"}),
    )
    .unwrap();
    assert_eq!(mcp.display_name, "server");
    assert_eq!(mcp.error, None);
    assert_eq!(mcp.plugin_name, None);
    let mcp_blank: McpSourceSummary = serde_json::from_value(json!({"name":"server","displayName":"","kind":"server","transport":"stdio","status":"connected"})).unwrap();
    assert_eq!(mcp_blank.display_name, "server");
}

#[test]
fn model_thinking_levels_use_vibes_factory_when_missing() {
    let model: ModelConfigView = serde_json::from_value(json!({"name":"m", "alias":"a", "thinking":ThinkingLevel::Off, "supportsImages":false, "displayName":"fixture-model"})).unwrap();
    assert_eq!(
        model.thinking_levels,
        vec![
            ThinkingLevel::Off,
            ThinkingLevel::Low,
            ThinkingLevel::Medium,
            ThinkingLevel::High,
            ThinkingLevel::Max
        ]
    );
}

#[test]
fn thinking_levels_match_the_vibe_wire_values_in_both_directions() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/thinking-levels.json")).unwrap();
    let levels = vec![
        ThinkingLevel::Off,
        ThinkingLevel::Low,
        ThinkingLevel::Medium,
        ThinkingLevel::High,
        ThinkingLevel::Max,
    ];
    assert_eq!(serde_json::to_value(&levels).unwrap(), fixture);
    assert_eq!(
        serde_json::from_value::<Vec<ThinkingLevel>>(fixture).unwrap(),
        levels
    );
}

#[test]
fn missing_required_fields_and_wrong_known_types_are_rejected() {
    assert!(serde_json::from_value::<SkillsListParams>(json!({})).is_err());
    assert!(serde_json::from_value::<SkillsListResponse>(json!({})).is_err());
    assert!(
        serde_json::from_value::<SkillsSetVersionParams>(
            json!({"sessionId":"s","name":"n","version":"4"})
        )
        .is_err()
    );
    assert!(
        serde_json::from_value::<SkillsSetEnabledParams>(
            json!({"sessionId":"s","name":"n","enabled":null})
        )
        .is_err()
    );
    assert!(
        serde_json::from_value::<SkillSummary>(
            json!({"name":"n","description":"d","prompt":"p","source":"invented"})
        )
        .is_err()
    );
    assert!(serde_json::from_value::<RuntimeMutationResponse>(json!({"runtime":{}})).is_err());
}
