//! A chat's Chrome session ends when its turn ends (task finished, nothing left open). When
//! the same chat later uses that ended session id (it asked the person something and goes on), the call is not
//! refused: the chat's session is reopened on the same window and tabs and the call runs, its receipt saying so.
//! Only ids recorded at a turn end (`ChromeControl::end_turn`) qualify: a session the person stopped, another chat's
//! session or an unknown id is refused as before (`gateway::validate_session`). The reopened call is checked like any
//! other (mode, stop generation); the reopen itself is this chat's own `chrome_session open` on its known profile.
use crate::core::Shared;
use serde_json::{Value, json};
use std::sync::{Arc, PoisonError};

/// What [`resolve`] decided.
pub(super) enum Resolved {
    /// The call's arguments, and the receipt's note when a turn-ended session was reopened.
    Ready(Value, Option<Value>),
    /// The reopen was refused with the card's diagnosis (`availability::refusal`): the call's answer.
    Refused(Value),
}

/// The call's arguments with the chat's current session id in place of a turn-ended one, and the note for its receipt.
pub(super) async fn resolve(
    shared: &Arc<Shared>,
    session: &str,
    tool: &str,
    mut args: Value,
) -> Result<Resolved, String> {
    let Some(id) = (tool == "chrome_execute")
        .then(|| args["sessionId"].as_str().map(str::to_owned))
        .flatten()
    else {
        return Ok(Resolved::Ready(args, None));
    };
    let chrome = &shared.chrome_control;
    let profile = chrome
        .work
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .profile_of(&id)
        .map(str::to_owned);
    let active = {
        let tasks = chrome.tasks.lock().unwrap_or_else(PoisonError::into_inner);
        if tasks.get(&id).map(String::as_str) == Some(session) {
            return Ok(Resolved::Ready(args, None));
        }
        let ended = chrome
            .turn_ended
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(session)
            .is_some_and(|ids| ids.contains(&id));
        if !ended {
            return Ok(Resolved::Ready(args, None));
        }
        tasks
            .iter()
            .find(|(id, owner)| {
                owner.as_str() == session
                    && profile.as_deref().is_some_and(|profile| {
                        chrome
                            .work
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .profile_of(id)
                            .is_some_and(|active| super::profile_ids::same(active, profile))
                    })
            })
            .map(|(id, _)| id.clone())
    };
    let current = if let Some(active) = active {
        active
    } else {
        let Some(profile) = profile else {
            return Ok(Resolved::Ready(args, None));
        };
        let opened = super::gateway::call_one(
            shared,
            session,
            "chrome_session",
            json!({"action":"open","profileId":profile,"purpose":"Reopened for this chat"}),
        )
        .await?;
        if opened["vwMcpResult"]["structuredContent"]
            .get("addon")
            .is_some()
            || opened["vwMcpResult"]["structuredContent"]["retryable"] == true
        {
            return Ok(Resolved::Refused(opened));
        }
        opened["vwMcpResult"]["structuredContent"]["sessionId"]
            .as_str()
            .map(str::to_owned)
            .ok_or("Not run: this Chrome session ended with an earlier turn and could not be reopened.")?
    };
    args["sessionId"] = json!(current);
    Ok(Resolved::Ready(
        args,
        Some(json!({"from":id,"sessionId":current})),
    ))
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

/// Discard an earlier turn's stopped open result while helper cleanup catches up. Never replay an action,
/// an unknown outcome, or a newly stopped session: the person's Stop still fences the current turn.
pub(super) async fn fresh(
    shared: &Arc<Shared>,
    session: &str,
    browser: super::agent_browser::Browser,
    pending: &super::person_stop::PendingCall,
    args: &Value,
    mut answer: super::owner::Answer,
) -> Result<super::owner::Answer, String> {
    for attempt in 0..3 {
        pending.finish(shared, session, browser, &answer)?;
        let body = &answer.value()["structuredContent"];
        if body["status"] == "unknown" {
            return Ok(answer);
        }
        let old = body["sessionId"].as_str().is_some_and(|id| {
            shared
                .chrome_control
                .work
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .stopped_before(id, session, pending.turn.as_deref())
        });
        let stopped = body["state"].as_str().is_some_and(|state| {
            state.eq_ignore_ascii_case("stopped") || state.eq_ignore_ascii_case("stopping")
        });
        if !old && !stopped {
            return Ok(answer);
        }
        if !stopped {
            return Err(format!(
                "{} reused an ended session. Check the browser before opening a fresh session.",
                browser.name()
            ));
        }
        if !old || attempt == 2 {
            return Err(format!(
                "Not run: {} returned a stopped session. Open a fresh session before acting.",
                browser.name()
            ));
        }
        super::reconnect::stopped(shared, session, browser)?;
        super::check::authorize(shared, session, "chrome_session", args)?;
        answer = super::login::send(shared, session, "chrome_session", args.clone()).await?;
    }
    unreachable!("the final stopped open returns a refusal")
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use std::sync::Arc;

    async fn chat_with_session() -> crate::Core {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let name = format!(
            "chrome-reopen-{}",
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );
        let core = crate::offline_core::start_with(&name, |config, dir| {
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
        let seen = super::super::test_support::answering(shared, helper("t2"));
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
        let seen = super::super::test_support::answering(shared, helper("t3"));
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

    /// A status poll can report the stop before the helper replies to the agent's own request.
    #[tokio::test]
    async fn the_agents_stop_allows_a_new_session_in_the_same_turn() {
        let core = chat_with_session().await;
        let shared = &core.shared;
        super::super::test_support::answering(shared, helper("t2"));
        super::super::gateway::call(
            shared,
            "s",
            "chrome_session",
            json!({"action":"open","profileId":"p"}),
        )
        .await
        .unwrap();
        let weak = Arc::downgrade(shared);
        let seen = shared.chrome_control.owner.answering(move |tool, args| {
            if tool == "chrome_session" && args["action"] == "stop" {
                let shared = weak.upgrade().unwrap();
                for state in ["stopping", "stopped"] {
                    super::super::owner::update_work_for_test(
                        &shared,
                        &json!({"structuredContent":{"sessions":[{"sessionId":"t2","state":state}]}}),
                    );
                }
            }
            helper("t3")(tool, args)
        });
        // Another chat cannot mark this session as an asked stop or reach the helper.
        assert!(
            super::super::gateway::call(
                shared,
                "o",
                "chrome_session",
                json!({"action":"stop","sessionId":"t2"}),
            )
            .await
            .is_err()
        );
        assert!(seen.lock().unwrap().is_empty());
        super::super::gateway::call(
            shared,
            "s",
            "chrome_session",
            json!({"action":"stop","sessionId":"t2"}),
        )
        .await
        .unwrap();
        let reopened = super::super::gateway::call(
            shared,
            "s",
            "chrome_session",
            json!({"action":"open","profileId":"p"}),
        )
        .await
        .unwrap_or_else(|reason| panic!("agent stop blocked the same turn: {reason}"));
        assert_eq!(
            reopened["vwMcpResult"]["structuredContent"]["sessionId"],
            "t3"
        );
        assert_eq!(opens(&seen).len(), 1);
        core.shutdown().await;
    }

    #[tokio::test]
    async fn a_refused_agent_stop_recovers_on_the_next_poll() {
        let core = chat_with_session().await;
        let shared = &core.shared;
        super::super::test_support::answering(shared, helper("t2"));
        super::super::gateway::call(
            shared,
            "s",
            "chrome_session",
            json!({"action":"open","profileId":"p"}),
        )
        .await
        .unwrap();
        shared.chrome_control.owner.answering(
            |_, _| json!({"isError":true,"structuredContent":{"error":{"code":"BUSY"}}}),
        );
        let refused = super::super::gateway::call(
            shared,
            "s",
            "chrome_session",
            json!({"action":"stop","sessionId":"t2"}),
        )
        .await
        .unwrap();
        assert_eq!(refused["vwMcpResult"]["isError"], true);
        assert_eq!(
            shared
                .chrome_control
                .work
                .lock()
                .unwrap()
                .state_of("t2", "s"),
            Some("stopping")
        );
        super::super::owner::update_work_for_test(
            shared,
            &json!({"structuredContent":{"sessions":[{"sessionId":"t2","state":"active"}]}}),
        );
        assert_eq!(
            shared
                .chrome_control
                .work
                .lock()
                .unwrap()
                .state_of("t2", "s"),
            Some("active")
        );
        assert_eq!(
            shared
                .chrome_control
                .tasks
                .lock()
                .unwrap()
                .get("t2")
                .map(String::as_str),
            Some("s")
        );
        assert!(!shared.chrome_control.stopped_by_person(
            "s",
            super::super::agent_browser::Browser::Chrome,
            None
        ));
        // Recovery must also restore detection of a later real Stop.
        super::super::owner::update_work_for_test(
            shared,
            &json!({"structuredContent":{"sessions":[{"sessionId":"t2","state":"stopping"}]}}),
        );
        assert!(shared.chrome_control.stopped_by_person(
            "s",
            super::super::agent_browser::Browser::Chrome,
            None
        ));
        core.shutdown().await;
    }

    /// After the person's Stop (the toolbar popup: the helper reports the
    /// chat's active session stopped without Vibe Work asking), every Chrome call of that chat that would act is
    /// refused for the rest of the turn, opening a new session included; the next turn starts normally.
    /// PREDICT (before the change): the open after the Stop runs and the agent goes on.
    #[tokio::test]
    async fn the_persons_stop_holds_for_the_rest_of_the_turn() {
        let stopped = format!(
            "Not run: you pressed Stop in {}. The turn is stopped; do not call browser tools again in this turn.",
            vw_vocabulary::texts::CHROME_NAME
        );
        let core = chat_with_session().await;
        let shared = &core.shared;
        let _seen = super::super::test_support::answering(shared, helper("t2"));
        super::super::gateway::call(
            shared,
            "s",
            "chrome_session",
            json!({"action":"open","profileId":"p"}),
        )
        .await
        .unwrap();
        // Stop is acknowledged asynchronously: a poll sees stopping before a later poll sees stopped.
        // The old code forgot who requested Stop as soon as the first poll changed the displayed state.
        super::super::owner::update_work_for_test(
            shared,
            &json!({"structuredContent":{"sessions":[{"sessionId":"t2","state":"stopping"}]}}),
        );
        tokio::time::pause();
        tokio::time::advance(std::time::Duration::from_secs(40)).await;
        tokio::time::resume();
        super::super::owner::update_work_for_test(
            shared,
            &json!({"structuredContent":{"sessions":[{"sessionId":"t2","state":"stopped"}]}}),
        );
        let seen = super::super::test_support::answering(shared, helper("t3"));
        for (tool, args) in [
            ("chrome_session", json!({"action":"open","profileId":"p"})),
            (
                "chrome_execute",
                json!({"sessionId":"t2","action":"page.read","args":{}}),
            ),
            (
                "chrome_execute",
                json!({"sessionId":"t2","steps":[{"action":"page.read","args":{}}]}),
            ),
        ] {
            assert_eq!(
                super::super::gateway::call(shared, "s", tool, args)
                    .await
                    .unwrap_err(),
                stopped,
                "{tool}"
            );
        }
        assert!(
            opens(&seen).is_empty(),
            "a new helper session must never be requested"
        );
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
            let seen = super::super::test_support::answering(shared, helper("t2"));
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
            let seen = super::super::test_support::answering(shared, helper("t3"));
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
        let seen = super::super::test_support::answering(shared, helper("t2"));
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
