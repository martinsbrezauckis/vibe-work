//! A chat's Chrome session ends when its turn ends (task finished, nothing left open). When
//! the same chat later uses that ended session id (it asked the person something and goes on), the call is not
//! refused: the chat's session is reopened on the same window and tabs and the call runs, its receipt saying so.
//! Only ids recorded at a turn end (`ChromeControl::end_turn`) qualify: a session the person stopped, another chat's
//! session or an unknown id is refused as before (`gateway::validate_session`). The reopened call is checked like any
//! other (mode, stop generation); the reopen itself is this chat's own `chrome_session open` on its known profile.
use crate::core::Shared;
use serde_json::{Value, json};
use std::sync::{Arc, PoisonError};

/// The call's arguments with the chat's current session id in place of a turn-ended one, and the note for its receipt.
pub(super) async fn resolve(
    shared: &Arc<Shared>,
    session: &str,
    tool: &str,
    mut args: Value,
) -> Result<(Value, Option<Value>), String> {
    let Some(id) = (tool == "chrome_execute")
        .then(|| args["sessionId"].as_str().map(str::to_owned))
        .flatten()
    else {
        return Ok((args, None));
    };
    let chrome = &shared.chrome_control;
    let active = {
        let tasks = chrome.tasks.lock().unwrap_or_else(PoisonError::into_inner);
        if tasks.get(&id).map(String::as_str) == Some(session) {
            return Ok((args, None));
        }
        let ended = chrome
            .turn_ended
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(session)
            .is_some_and(|ids| ids.contains(&id));
        if !ended {
            return Ok((args, None));
        }
        tasks
            .iter()
            .find(|(_, owner)| owner.as_str() == session)
            .map(|(id, _)| id.clone())
    };
    let current = if let Some(active) = active {
        active
    } else {
        let Some(profile) = chrome
            .chat_profiles
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(session)
            .cloned()
        else {
            return Ok((args, None));
        };
        let opened = super::gateway::call_one(
            shared,
            session,
            "chrome_session",
            json!({"action":"open","profileId":profile,"purpose":"Reopened for this chat"}),
        )
        .await?;
        opened["vwMcpResult"]["structuredContent"]["sessionId"]
            .as_str()
            .map(str::to_owned)
            .ok_or("Not run: this Chrome session ended with an earlier turn and could not be reopened.")?
    };
    args["sessionId"] = json!(current);
    Ok((args, Some(json!({"from":id,"sessionId":current}))))
}

/// The receipt says the session was reopened (structured and in the text the model reads).
pub(super) fn note(answer: &mut Value, reopened: &Value) {
    let result = &mut answer["vwMcpResult"];
    if result["structuredContent"].is_object() {
        result["structuredContent"]["sessionReopened"] = reopened.clone();
    }
    if let Some(content) = result["content"].as_array_mut() {
        for item in content.iter_mut().filter(|item| item["type"] == "text") {
            let text = item["text"].as_str().unwrap_or_default();
            item["text"] = json!(match serde_json::from_str::<Value>(text) {
                Ok(mut body) if body.is_object() => {
                    body["sessionReopened"] = reopened.clone();
                    body.to_string()
                }
                _ => format!("{}\n{text}", json!({"sessionReopened":reopened})),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use std::sync::Arc;

    async fn chat_with_session() -> crate::Core {
        let core = crate::offline_core::start_with("chrome-reopen", |config, dir| {
            config.chrome_control.package = Some(dir.join("chrome-package"));
        })
        .await;
        let shared = &core.shared;
        for chat in ["s", "o"] {
            let _ = shared
                .store
                .load(
                    json!({"session":{"id":chat},"history":[],"turns":[]}),
                    vw_session::LoadKind::New,
                )
                .unwrap();
            shared.access.observe_request(
                "session/agent/update",
                chat,
                &json!({"agentName":"auto-approve"}),
            );
        }
        core
    }

    fn helper(opened: &'static str) -> impl Fn(&str, &Value) -> Value + Send + 'static {
        move |tool, args| {
            if tool == "chrome_session" && args["action"] == "open" {
                json!({"isError":false,"content":[{"type":"text","text":"opened"}],
                    "structuredContent":{"sessionId":opened,"state":"active","profileId":"p"}})
            } else {
                json!({"isError":false,"content":[{"type":"text","text":"done"}],
                    "structuredContent":{"operationId":"op","status":"completed","sessionId":args["sessionId"],"data":{}}})
            }
        }
    }

    fn opens(seen: &Arc<std::sync::Mutex<Vec<(String, Value)>>>) -> Vec<Value> {
        seen.lock()
            .unwrap()
            .iter()
            .filter(|(t, a)| t == "chrome_session" && a["action"] == "open")
            .map(|(_, a)| a.clone())
            .collect()
    }

    /// PREDICT (before the change): the chat's own session id, used after its turn ended, is refused with "belongs to
    /// another chat or an earlier helper launch"; after it, the call runs in a reopened session of the same chat.
    #[tokio::test]
    async fn a_session_ended_with_its_turn_is_reopened_for_the_same_chat() {
        let core = chat_with_session().await;
        let shared = &core.shared;
        let seen = shared.chrome_control.owner.answering(helper("t2"));
        let first = super::super::gateway::call(
            shared,
            "s",
            "chrome_session",
            json!({"action":"open","profileId":"p"}),
        )
        .await
        .unwrap();
        assert_eq!(first["vwMcpResult"]["structuredContent"]["sessionId"], "t2");
        // The helper's first session for this chat is t2; give the next open another id.
        drop(seen);
        let seen = shared.chrome_control.owner.answering(helper("t3"));
        // The turn ends (pump.rs: completed turn).
        super::super::end_turn(shared, "s");
        for _ in 0..2 {
            let answer = super::super::gateway::call(
                shared,
                "s",
                "chrome_execute",
                json!({"sessionId":"t2","action":"page.read","args":{}}),
            )
            .await;
            let answer = answer.unwrap_or_else(|refusal| panic!("refused: {refusal}"));
            let body = &answer["vwMcpResult"]["structuredContent"];
            assert_eq!(body["status"], "completed", "{answer}");
            assert_eq!(
                body["sessionReopened"],
                json!({"from":"t2","sessionId":"t3"}),
                "{answer}"
            );
            assert!(
                answer["vwMcpResult"]["content"][0]["text"]
                    .as_str()
                    .unwrap()
                    .contains("sessionReopened"),
                "{answer}"
            );
        }
        // One reopen for both calls, on the chat's own profile; the call itself ran in the reopened session.
        assert_eq!(opens(&seen).len(), 1);
        assert_eq!(opens(&seen)[0]["profileId"], "p");
        assert!(
            seen.lock()
                .unwrap()
                .iter()
                .any(|(t, a)| t == "chrome_execute" && a["sessionId"] == "t3")
        );
        // A batch with the ended id runs too.
        let batch = super::super::gateway::call(
            shared,
            "s",
            "chrome_execute",
            json!({"sessionId":"t2","steps":[{"action":"page.read","args":{}}]}),
        )
        .await
        .unwrap();
        assert_eq!(
            batch["vwMcpResult"]["structuredContent"]["status"], "completed",
            "{batch}"
        );
        // Another chat never gets this chat's session, ended or not.
        let other = super::super::gateway::call(
            shared,
            "o",
            "chrome_execute",
            json!({"sessionId":"t2","action":"page.read","args":{}}),
        )
        .await;
        assert_eq!(
            other.unwrap_err(),
            "Not run: this Chrome session belongs to another chat."
        );
        // An id no chat ever had says so (Mistral passed a made-up "default" in T34/T35).
        let unknown = super::super::gateway::call(
            shared,
            "s",
            "chrome_execute",
            json!({"sessionId":"default","action":"page.read","args":{}}),
        )
        .await;
        assert_eq!(
            unknown.unwrap_err(),
            "Not run: no Chrome session with this id; open one with chrome_session."
        );
        assert_eq!(opens(&seen).len(), 1);
        core.shutdown().await;
    }

    /// After the person's Stop (the toolbar popup: the helper reports the
    /// chat's active session stopped without Vibe Work asking), every Chrome call of that chat that would act is
    /// refused for the rest of the turn, opening a new session included; the next turn starts normally.
    /// PREDICT (before the change): the open after the Stop runs and the agent goes on.
    #[tokio::test]
    async fn the_persons_stop_holds_for_the_rest_of_the_turn() {
        const STOPPED: &str = "Not run: the person stopped Chrome for this turn. Continue without Chrome in this turn; Chrome works again in the next one.";
        let core = chat_with_session().await;
        let shared = &core.shared;
        let _seen = shared.chrome_control.owner.answering(helper("t2"));
        super::super::gateway::call(
            shared,
            "s",
            "chrome_session",
            json!({"action":"open","profileId":"p"}),
        )
        .await
        .unwrap();
        // The person presses Stop in Chrome: the helper's status shows the session stopped (no core stop asked).
        super::super::owner::update_work_for_test(
            shared,
            &json!({"structuredContent":{"sessions":[{"sessionId":"t2","state":"stopped"}]}}),
        );
        for (tool, args) in [
            (
                "chrome_execute",
                json!({"sessionId":"t2","action":"page.read","args":{}}),
            ),
            ("chrome_session", json!({"action":"open","profileId":"p"})),
            (
                "chrome_execute",
                json!({"sessionId":"t2","steps":[{"action":"page.read","args":{}}]}),
            ),
        ] {
            assert_eq!(
                super::super::gateway::call(shared, "s", tool, args)
                    .await
                    .unwrap_err(),
                STOPPED,
                "{tool}"
            );
        }
        // Reading the session stays possible; another chat is not affected.
        assert!(
            super::super::gateway::call(
                shared,
                "o",
                "chrome_session",
                json!({"action":"open","profileId":"p"})
            )
            .await
            .is_ok()
        );
        // The turn ends: the next turn starts normally.
        super::super::end_turn(shared, "s");
        assert!(
            super::super::gateway::call(
                shared,
                "s",
                "chrome_session",
                json!({"action":"open","profileId":"p"})
            )
            .await
            .is_ok()
        );
        core.shutdown().await;
    }

    /// An explicit Stop (Chrome-side or the app's) revokes the chat's turn-ended ids, so an older id cannot
    /// reopen Chrome after the person stopped its successor. PREDICT (before the change): turn 1 opens t2 and ends,
    /// turn 2 reuses t2 (reopened as t3), the person stops it, turn 3 reuses t2 and a new session opens.
    #[tokio::test]
    async fn an_explicit_stop_revokes_the_turn_ended_ids() {
        for app_stop in [false, true] {
            let core = chat_with_session().await;
            let shared = &core.shared;
            let seen = shared.chrome_control.owner.answering(helper("t2"));
            super::super::gateway::call(
                shared,
                "s",
                "chrome_session",
                json!({"action":"open","profileId":"p"}),
            )
            .await
            .unwrap();
            super::super::end_turn(shared, "s");
            drop(seen);
            let seen = shared.chrome_control.owner.answering(helper("t3"));
            let reopened = super::super::gateway::call(
                shared,
                "s",
                "chrome_execute",
                json!({"sessionId":"t2","action":"page.read","args":{}}),
            )
            .await
            .unwrap();
            assert_eq!(
                reopened["vwMcpResult"]["structuredContent"]["sessionReopened"]["sessionId"],
                "t3"
            );
            if app_stop {
                crate::stops::stopped(shared, "s");
            } else {
                super::super::owner::update_work_for_test(
                    shared,
                    &json!({"structuredContent":{"sessions":[{"sessionId":"t3","state":"stopped"}]}}),
                );
            }
            super::super::end_turn(shared, "s");
            let later = super::super::gateway::call(
                shared,
                "s",
                "chrome_execute",
                json!({"sessionId":"t2","action":"page.read","args":{}}),
            )
            .await;
            assert!(later.is_err(), "app_stop={app_stop}: {later:?}");
            assert_eq!(opens(&seen).len(), 1, "app_stop={app_stop}");
            // A new session of the chat in a later turn is fine.
            assert!(
                super::super::gateway::call(
                    shared,
                    "s",
                    "chrome_session",
                    json!({"action":"open","profileId":"p"})
                )
                .await
                .is_ok()
            );
            core.shutdown().await;
        }
    }

    /// The person's Stop is not a turn end: that session is not reopened, and the refusal says what happened.
    #[tokio::test]
    async fn a_session_the_person_stopped_is_not_reopened() {
        let core = chat_with_session().await;
        let shared = &core.shared;
        let seen = shared.chrome_control.owner.answering(helper("t2"));
        super::super::gateway::call(
            shared,
            "s",
            "chrome_session",
            json!({"action":"open","profileId":"p"}),
        )
        .await
        .unwrap();
        crate::stops::stopped(shared, "s");
        let refused = super::super::gateway::call(
            shared,
            "s",
            "chrome_execute",
            json!({"sessionId":"t2","action":"page.read","args":{}}),
        )
        .await;
        assert_eq!(
            refused.unwrap_err(),
            "Not run: this Chrome session was stopped."
        );
        assert_eq!(opens(&seen).len(), 1);
        core.shutdown().await;
    }
}
