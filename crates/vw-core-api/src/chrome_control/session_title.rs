//! Host-owned chat titles for the helper's optional session title contract. Called when opening Chrome sessions.
//! Older helpers/extensions retain the original arguments; website text and model arguments cannot supply the chat title.
use super::agent_browser::Browser;
use crate::core::Shared;
use serde_json::{Value, json};

pub(super) async fn apply(
    shared: &Shared,
    session: &str,
    browser: Browser,
    args: &mut Value,
) -> Result<(), String> {
    let generation = crate::stops::generation(shared, session);
    let turn = super::person_stop::active_turn(shared, session);
    let status = shared
        .chrome_control
        .owner
        .call("chrome_status", json!({}))
        .await;
    if generation != crate::stops::generation(shared, session)
        || turn != super::person_stop::active_turn(shared, session)
        || shared.store.view(session).is_none()
    {
        return Err(super::person_stop::ended(browser));
    }
    let status = &status.value()["structuredContent"];
    super::profile_seeding::resolve(shared, session, args, status)?;
    super::profile_agent::fresh(shared, args, status)?;
    let profile = args["profileId"].as_str().and_then(|selected| {
        status["profiles"].as_array()?.iter().find(|profile| {
            profile["profileId"]
                .as_str()
                .is_some_and(|id| super::profile_ids::same(id, selected))
        })
    });
    if status["supportsSessionTitle"] == true
        && profile.is_some_and(|profile| {
            profile["connected"] == true
                && profile["extensionCapabilities"]
                    .as_array()
                    .is_some_and(|caps| caps.iter().any(|cap| cap == "session.title"))
        })
    {
        args["_chatTitle"] = json!(shared.names.display_title(&shared.store, session));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROFILE: &str = "12345678-1234-1234-1234-123456789abc";

    fn title_support_cases() -> [(&'static str, bool, Value, bool); 11] {
        let profile = |id: &str, connected: bool, capabilities: Value| json!([{"profileId":id,"connected":connected,"extensionCapabilities":capabilities}]);
        [
            (
                "old helper",
                false,
                profile(PROFILE, true, json!(["session.title"])),
                false,
            ),
            (
                "old extension",
                true,
                profile(PROFILE, true, json!([])),
                false,
            ),
            (
                "missing capabilities",
                true,
                profile(PROFILE, true, Value::Null),
                false,
            ),
            (
                "other capability",
                true,
                profile(PROFILE, true, json!(["page.read.summary"])),
                false,
            ),
            (
                "title support",
                true,
                profile(PROFILE, true, json!(["session.title"])),
                true,
            ),
            (
                "disconnected",
                true,
                profile(PROFILE, false, json!(["session.title"])),
                false,
            ),
            (
                "unknown profile",
                true,
                profile("other", true, json!(["session.title"])),
                false,
            ),
            ("empty profiles", true, json!([]), false),
            ("absent profiles", true, Value::Null, false),
            (
                "Chrome alias",
                true,
                profile(&format!("chrome:{PROFILE}"), true, json!(["session.title"])),
                true,
            ),
            (
                "Edge is distinct",
                true,
                profile(&format!("edge:{PROFILE}"), true, json!(["session.title"])),
                false,
            ),
        ]
    }

    #[tokio::test]
    async fn model_title_is_refused_before_any_helper_call() {
        let core = crate::offline_core::start_with("chrome-title-authority", |config, dir| {
            config.chrome_control.package = Some(dir.join("chrome-package"));
        })
        .await;
        let _ = core
            .shared
            .store
            .load(
                json!({"session":{"id":"s","title":"Real chat"},"history":[],"turns":[]}),
                vw_session::LoadKind::New,
            )
            .unwrap();
        core.shared.access.observe_request(
            "session/agent/update",
            "s",
            &json!({"agentName":"auto-approve"}),
        );
        let seen = super::super::test_support::answering(&core.shared, |tool, args| {
            if tool == "chrome_status" {
                json!({"structuredContent":{}})
            } else {
                json!({"structuredContent":{"sessionId":"t","profileId":"p","state":"active","title":args["title"]}})
            }
        });
        let result = super::super::gateway::call(
            &core.shared,
            "s",
            "chrome_session",
            json!({"action":"open","profileId":"p","title":"Untrusted title"}),
        )
        .await;
        assert!(result.is_err(), "{result:?}");
        assert!(seen.lock().unwrap().is_empty());
        core.shutdown().await;
    }

    #[tokio::test]
    async fn opening_uses_the_chat_title_only_when_the_helper_and_profile_accept_it() {
        let core = crate::offline_core::start_with("chrome-session-title", |config, dir| {
            config.chrome_control.package = Some(dir.join("chrome-package"));
        })
        .await;
        let _ = core
            .shared
            .store
            .load(
                json!({"session":{"id":"s","title":"Quarterly report"},"history":[],"turns":[]}),
                vw_session::LoadKind::New,
            )
            .unwrap();
        core.shared.access.observe_request(
            "session/agent/update",
            "s",
            &json!({"agentName":"auto-approve"}),
        );
        for (case, supported, profiles, expected) in title_support_cases() {
            let seen = super::super::test_support::answering(&core.shared, move |tool, args| {
                if tool == "chrome_status" {
                    let mut status =
                        json!({"structuredContent":{"supportsSessionTitle":supported}});
                    if !profiles.is_null() {
                        status["structuredContent"]["profiles"] = profiles.clone();
                    }
                    status
                } else {
                    json!({"structuredContent":{"sessionId":"t","profileId":args["profileId"],"state":"active"}})
                }
            });
            let unavailable = matches!(
                case,
                "disconnected" | "unknown profile" | "empty profiles" | "Edge is distinct"
            );
            let result = super::super::gateway::call(
                &core.shared,
                "s",
                "chrome_session",
                json!({"action":"open","profileId":PROFILE,"purpose":"Model task"}),
            )
            .await;
            if unavailable {
                assert!(result.is_err(), "{case}: {result:?}");
                assert!(
                    !seen
                        .lock()
                        .unwrap()
                        .iter()
                        .any(|(tool, _)| tool == "chrome_session")
                );
                continue;
            }
            let result = result.unwrap();
            assert_eq!(result["vwMcpResult"]["structuredContent"]["sessionId"], "t");
            let calls = seen.lock().unwrap();
            let args = &calls
                .iter()
                .find(|(tool, args)| tool == "chrome_session" && args["action"] == "open")
                .unwrap()
                .1;
            assert_eq!(
                args.get("_chatTitle"),
                expected.then_some(&json!("Quarterly report")),
                "{case}"
            );
            assert_eq!(args["purpose"], "Model task");
        }
        core.shutdown().await;
    }

    #[tokio::test]
    async fn stop_during_title_negotiation_never_opens_a_session() {
        let core = crate::offline_core::start_with("chrome-title-stop", |config, dir| {
            config.chrome_control.package = Some(dir.join("chrome-package"));
        })
        .await;
        let _ = core
            .shared
            .store
            .load(
                json!({"session":{"id":"s","title":"Report"},"history":[],"turns":[]}),
                vw_session::LoadKind::New,
            )
            .unwrap();
        core.shared.access.observe_request(
            "session/agent/update",
            "s",
            &json!({"agentName":"auto-approve"}),
        );
        let shared = core.shared.clone();
        let seen = super::super::test_support::answering(&core.shared, move |tool, _| {
            if tool == "chrome_status" {
                crate::stops::stopped(&shared, "s");
                json!({"structuredContent":{"supportsSessionTitle":true}})
            } else {
                json!({"structuredContent":{"sessionId":"t","profileId":"p","state":"active"}})
            }
        });
        let result = super::super::gateway::call(
            &core.shared,
            "s",
            "chrome_session",
            json!({"action":"open","profileId":"p"}),
        )
        .await;
        assert!(result.is_err(), "{result:?}");
        assert!(
            !seen
                .lock()
                .unwrap()
                .iter()
                .any(|(tool, args)| tool == "chrome_session" && args["action"] == "open")
        );
        core.shutdown().await;
    }
}
