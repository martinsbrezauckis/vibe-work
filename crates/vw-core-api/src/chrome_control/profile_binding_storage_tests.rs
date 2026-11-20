//! Browser-binding disk failure regressions through the checked gateway.
use super::*;

/// PREDICT: after a real helper open, a failed chat-profiles.json replacement must
/// retain its in-memory lease, refuse another external profile, and allow a
/// positive old-session end to release the chat for a new profile.
#[tokio::test]
async fn a_failed_binding_save_after_open_keeps_the_live_chat_locked() {
    let core = connected("profile-binding-save-fails-after-open").await;
    let path = core
        .shared
        .chrome_control
        .dir()
        .unwrap()
        .join("chat-profiles.json");
    std::fs::create_dir(&path).unwrap();

    let first = open(&core, "chrome:work").await.unwrap();
    assert!(
        first.to_string().contains("\"sessionId\":\"task\""),
        "{first}"
    );
    assert_open_recorded(&core);
    assert_eq!(
        super::super::profile_bindings::selected(&core.shared.chrome_control, "s").as_deref(),
        Some("chrome:work"),
        "the successful helper open must remain bound in memory"
    );
    let refusal = open(&core, "edge:work").await.unwrap_err();
    assert!(
        refusal.contains("Close that browser session first")
            && refusal.contains("built-in browser in parallel"),
        "{refusal}"
    );

    super::super::owner::update_work_for_test(
        &core.shared,
        &json!({"structuredContent":{"version":1,"clientId":"vibework-owner","profiles":[],"sessions":[{"sessionId":"task","clientSessionId":"s","profileId":"chrome:work","state":"stopped","capture":"off"}]}}),
    );
    assert_eq!(
        super::super::profile_bindings::selected(&core.shared.chrome_control, "s"),
        None,
        "a positive old-session end must release the in-memory barrier even while disk remains unwritable"
    );
    assert!(
        super::super::profile_bindings::check(&core.shared.chrome_control, "s", "edge:work")
            .is_ok()
    );
    std::fs::remove_dir(&path).unwrap();
    test_support::answering(&core.shared, |tool, args| {
        if tool == "chrome_status" {
            json!({"structuredContent":{"version":1,"clientId":"vibework-owner","profiles":[{"profileId":"edge:work","name":"Work","connected":true}],"sessions":[{"sessionId":"task","clientSessionId":"s","profileId":"chrome:work","state":"stopped","capture":"off"}]}})
        } else if args["action"] == "open" {
            json!({"structuredContent":{"sessionId":"edge-task","profileId":"edge:work","state":"active"}})
        } else {
            json!({"structuredContent":{}})
        }
    });
    open(&core, "edge:work").await.unwrap();
    assert_eq!(
        super::super::profile_bindings::selected(&core.shared.chrome_control, "s").as_deref(),
        Some("edge:work")
    );
    assert_eq!(
        super::super::profile_bindings::Bindings::load(core.shared.chrome_control.dir()).chats()["s"],
        "edge:work",
        "the next successful save must persist the current binding"
    );
    std::fs::rename(&path, path.with_extension("json.edge-before-fault")).unwrap();
    std::fs::create_dir(&path).unwrap();
    super::super::owner::update_work_for_test(
        &core.shared,
        &json!({"structuredContent":{"version":1,"clientId":"vibework-owner","profiles":[],"sessions":[{"sessionId":"edge-task","clientSessionId":"s","profileId":"edge:work","state":"disconnected","capture":"unknown","suspensionReason":"WINDOW_CLOSED","target":null}]}}),
    );
    assert_eq!(
        super::super::profile_bindings::selected(&core.shared.chrome_control, "s"),
        None,
        "a positively fenced window close must also release the binding while disk is unwritable"
    );
    std::fs::remove_dir(&path).unwrap();
    core.shared
        .chrome_control
        .bindings
        .lock()
        .unwrap()
        .save(core.shared.chrome_control.dir())
        .unwrap();
    assert!(
        !super::super::profile_bindings::Bindings::load(core.shared.chrome_control.dir())
            .chats()
            .contains_key("s"),
        "the next successful save must persist the closed binding's removal"
    );
    core.shutdown().await;
}

#[allow(clippy::unwrap_used, reason = "test checks task and Work mutex state")]
fn assert_open_recorded(core: &crate::Core) {
    assert!(
        core.shared
            .chrome_control
            .tasks
            .lock()
            .unwrap()
            .contains_key("task"),
        "the successful helper open must enter the task ledger"
    );
    assert!(
        !core
            .shared
            .chrome_control
            .work
            .lock()
            .unwrap()
            .releases("s")
            .is_empty(),
        "Stop must retain a release target after a failed disk save"
    );
}

/// PREDICT: an unknown first open retains its fail-closed in-memory barrier even
/// when the storage replacement fails, for both known and absent helper IDs.
#[tokio::test]
async fn an_uncertain_open_remains_fenced_when_its_save_fails() {
    let core = connected("profile-uncertain-save-fails").await;
    let path = core
        .shared
        .chrome_control
        .dir()
        .unwrap()
        .join("chat-profiles.json");
    std::fs::create_dir(&path).unwrap();
    for (chat, id) in [("s", Some("unknown-task")), ("other", None)] {
        super::super::profile_bindings::mark_uncertain(
            &core.shared.chrome_control,
            chat,
            id,
            Some("chrome:work"),
        );
        assert!(super::super::profile_bindings::uncertain(
            &core.shared.chrome_control,
            chat
        ));
        assert_eq!(
            super::super::profile_bindings::check(&core.shared.chrome_control, chat, "edge:work")
                .unwrap_err(),
            vw_vocabulary::texts::BROWSER_PROFILE_OPEN_UNCONFIRMED
        );
    }
    std::fs::remove_dir(&path).unwrap();
    core.shared
        .chrome_control
        .bindings
        .lock()
        .unwrap()
        .save(core.shared.chrome_control.dir())
        .unwrap();
    let loaded = super::super::profile_bindings::Bindings::load(core.shared.chrome_control.dir());
    assert!(loaded.check("s", "edge:work").is_err());
    assert!(loaded.check("other", "edge:work").is_err());
    core.shutdown().await;
}
