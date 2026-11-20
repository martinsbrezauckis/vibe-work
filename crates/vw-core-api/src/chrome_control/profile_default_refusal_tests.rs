//! An open that names another profile than the person's default in a chat with no profile yet: the agent learns why
//! it was refused and which exact profile ID it may use.
//!
//! PREDICT (written before the change): both opens are refused "Work is the default for new chats; no action ran.
//! Use its profile ID, or ask the user to change the default profile in Settings > Browser." The text tells the agent
//! to use "its" ID without naming it, never says that an agent cannot move a new chat off the person's default, and
//! answers a profile name (not an ID) as if it were a known profile, so an agent that already used the requested
//! profile's ID repeats it.
#![allow(
    clippy::unwrap_used,
    reason = "test fixtures and held calls must exist for the test to mean anything"
)]
use super::test_support;
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};

async fn core(name: &str) -> crate::Core {
    let helper = json!({"profiles":[
        {"profileId":"chrome:work","connected":true},
        {"profileId":"chrome:personal","connected":true}
    ]});
    let core = crate::offline_core::start_with(name, |config, dir| {
        config.chrome_control.package = Some(dir.join("package"));
        config.agent_tools = Some(crate::AgentToolsConfig::new(
            dir.join("tools"),
            dir.join("data"),
            None,
        ));
    })
    .await;
    test_support::chat(&core.shared, "s");
    let reported = helper.clone();
    test_support::answering(&core.shared, move |tool, args| {
        if tool == "chrome_status" {
            return json!({"structuredContent":reported});
        }
        if args["action"] == "open" {
            let task = format!("task-{}", args["profileId"].as_str().unwrap_or_default());
            return json!({"structuredContent":{"sessionId":task,"profileId":args["profileId"],"state":"active"}});
        }
        json!({"structuredContent":{}})
    });
    test_support::observe_status(&core.shared, &helper);
    {
        let mut saved = core.shared.chrome_control.profiles.lock().unwrap();
        for (id, name) in [("chrome:work", "Work"), ("chrome:personal", "Personal")] {
            saved.records.get_mut(id).unwrap().browser_name = Some(name.into());
        }
        saved.defaults.insert("chrome".into(), "chrome:work".into());
    }
    core
}

async fn open(shared: &Arc<crate::core::Shared>, profile: &str) -> String {
    let result = tokio::time::timeout(
        Duration::from_secs(10),
        super::gateway::call(
            shared,
            "s",
            "chrome_session",
            json!({"action":"open","profileId":profile}),
        ),
    )
    .await
    .expect("the default decides without a card");
    match result {
        Err(text) => text,
        Ok(value) => panic!("the open must be refused: {value}"),
    }
}

#[tokio::test]
async fn a_known_non_default_profile_is_refused_with_the_reason_and_the_default_id() {
    let core = core("default-refusal-known").await;
    let text = open(&core.shared, "chrome:personal").await;
    assert!(
        text.contains("Personal is not the default for new chats"),
        "{text}"
    );
    assert!(
        text.contains("an agent cannot move a new chat to another profile"),
        "{text}"
    );
    assert!(text.contains("Work [chrome:work]"), "{text}");
    assert!(!text.contains("Use its profile ID"), "{text}");
    core.shutdown().await;
}

#[tokio::test]
async fn a_profile_name_instead_of_an_id_is_named_as_unknown() {
    let core = core("default-refusal-name").await;
    let text: Value = json!(open(&core.shared, "chrome:Personal profile").await);
    let text = text.as_str().unwrap();
    assert!(text.contains("is not a known Chrome profile ID"), "{text}");
    assert!(text.contains("Work [chrome:work]"), "{text}");
    core.shutdown().await;
}
