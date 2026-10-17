//! Replays tagged-model request/result fixtures through a fake transport.
//! Calls the public client only; exact JSON equality falsifies method/field drift.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{collections::VecDeque, sync::Mutex};

use serde::Deserialize;
use serde_json::Value;
use vw_skills::{
    SkillsCatalogParams, SkillsClient, SkillsConvertLocalParams, SkillsDetailParams,
    SkillsImportParams, SkillsInstalledParams, SkillsListParams, SkillsRemoveParams,
    SkillsSetAliasParams, SkillsSetEnabledParams, SkillsSetLatestParams, SkillsSetVersionParams,
    SkillsTransport, SkillsUpdatesParams, SkillsVersionsParams, TransportError,
};

#[derive(Deserialize)]
struct Fixture {
    method: String,
    params: Value,
    result: Value,
}

struct Replay(Mutex<VecDeque<Fixture>>);

impl SkillsTransport for Replay {
    fn request(
        &self,
        method: &str,
        params: Value,
    ) -> impl std::future::Future<Output = Result<Value, TransportError>> + Send {
        let fixture = self
            .0
            .lock()
            .unwrap()
            .pop_front()
            .expect("Unexpected request");
        assert_eq!(method, fixture.method);
        assert_eq!(params, fixture.params);
        std::future::ready(Ok(fixture.result))
    }
}

fn replay(json: &str) -> (SkillsClient<Replay>, Value, Value) {
    let fixture: Fixture = serde_json::from_str(json).unwrap();
    let params = fixture.params.clone();
    let result = fixture.result.clone();
    (
        SkillsClient::new(Replay(Mutex::new(VecDeque::from([fixture])))),
        params,
        result,
    )
}

macro_rules! replay_test {
    ($name:ident, $file:literal, $method:ident, $params:ty) => {
        #[tokio::test]
        async fn $name() {
            let (client, params, result) = replay(include_str!($file));
            let params: $params = serde_json::from_value(params).unwrap();
            let response = client.$method(&params).await.unwrap();
            assert_eq!(serde_json::to_value(response).unwrap(), result);
        }
    };
}

replay_test!(list, "fixtures/list.json", list, SkillsListParams);
replay_test!(
    installed,
    "fixtures/installed.json",
    installed,
    SkillsInstalledParams
);
replay_test!(
    catalog,
    "fixtures/catalog.json",
    catalog,
    SkillsCatalogParams
);
replay_test!(
    versions,
    "fixtures/versions.json",
    versions,
    SkillsVersionsParams
);
replay_test!(
    updates,
    "fixtures/updates.json",
    updates,
    SkillsUpdatesParams
);
replay_test!(detail, "fixtures/detail.json", detail, SkillsDetailParams);
replay_test!(import, "fixtures/import.json", import, SkillsImportParams);
replay_test!(
    set_version,
    "fixtures/setVersion.json",
    set_version,
    SkillsSetVersionParams
);
replay_test!(
    set_latest,
    "fixtures/setLatest.json",
    set_latest,
    SkillsSetLatestParams
);
replay_test!(
    set_alias,
    "fixtures/setAlias.json",
    set_alias,
    SkillsSetAliasParams
);
replay_test!(remove, "fixtures/remove.json", remove, SkillsRemoveParams);
replay_test!(
    set_enabled,
    "fixtures/setEnabled.json",
    set_enabled,
    SkillsSetEnabledParams
);
replay_test!(
    convert_local,
    "fixtures/convertLocal.json",
    convert_local,
    SkillsConvertLocalParams
);
