//! Native confirmation tokens for privileged gate calls.
//! PREDICT: only the confirmed method and params pass once; decline, expiry, and forgery send nothing.

mod common;

use std::time::Duration;

use common::gate::{Frame, rig, rig_with};
use serde_json::{Value, json};
use vw_gate::{Class, Confirmation, ConfirmationToken, GateError, TokenFault, classified_methods};

/// Realistic, type-valid params for one confirmed method of each family.
fn p2_cases() -> Vec<(&'static str, Value)> {
    vec![
        (
            "mcp_catalog/add",
            json!({"sessionId": "s1", "url": "https://mcp.example.com/sse", "name": "ex"}),
        ),
        ("mcp/add", json!({"url": "https://mcp.example.com/sse"})),
        (
            "agents/install",
            json!({"sessionId": "s1", "agentName": "evil"}),
        ),
        (
            "agents/uninstall",
            json!({"sessionId": "s1", "agentName": "evil"}),
        ),
        (
            "connector_catalog/auth/request",
            json!({"sessionId": "s1", "alias": "atlassian"}),
        ),
        (
            "config/proxy/write",
            json!({"sessionId": "s1", "changes": {"HTTPS_PROXY": "http://attacker.example:8080"}}),
        ),
        (
            "skills/import",
            json!({"sessionId": "s1", "skillId": "sk-1"}),
        ),
        (
            "vibeCode/teleport/start",
            json!({"sessionId": "s1", "pickerId": "p", "operationId": "o", "projectId": "pr"}),
        ),
        (
            "vibeCode/projects/create",
            json!({"sessionId": "s1", "pickerId": "p", "name": "n", "defaultBranch": "main"}),
        ),
        (
            "projectLinks/create",
            json!({"rootPath": "/work/repo", "name": "n", "defaultBranch": "main"}),
        ),
    ]
}

fn assert_required(r: Result<vw_gate::Reply, GateError>, what: &str) {
    assert!(
        matches!(r, Err(GateError::ConfirmationRequired { .. })),
        "{what}: {r:?}"
    );
}

/// PREDICT: with no token, each confirmed method is refused with ConfirmationRequired; the confirmer is
/// never consulted by `call`, and nothing is sent.
#[tokio::test]
async fn p2_without_token_is_refused() {
    let rig = rig();
    for (method, params) in p2_cases() {
        assert_required(rig.gate.call(method, params, None).await, method);
    }
    assert!(rig.transport.frames().is_empty());
    assert!(rig.confirmer.shown().is_empty());
}

/// PREDICT: every confirmed row of the table is refused with any params and no token (even params that
/// fail type validation), and none of them ever reaches the transport.
#[tokio::test]
async fn every_p2_method_is_refused_without_a_token() {
    let rig = rig();
    let p2: Vec<&str> = classified_methods()
        .filter(|(_, c)| *c == Class::Confirmed)
        .map(|(m, _)| m)
        .collect();
    assert!(p2.len() >= 25, "suspiciously few confirmed methods: {p2:?}");
    for method in p2 {
        let r = rig.gate.call(method, json!({}), None).await;
        assert!(r.is_err(), "{method}: {r:?}");
    }
    assert!(rig.transport.frames().is_empty());
}

/// PREDICT: confirm -> token -> call forwards the request byte-exact, once; the user was shown the
/// exact method and params.
#[tokio::test]
async fn confirmed_call_is_forwarded_once_with_exact_params() {
    for (method, params) in p2_cases() {
        let rig = rig();
        let token = rig
            .gate
            .request_confirmation(method, &params)
            .await
            .unwrap();
        let shown = rig.confirmer.shown();
        assert_eq!(shown.len(), 1, "{method}");
        assert_eq!(shown[0].method, method);
        assert_eq!(shown[0].params, params);
        assert!(!shown[0].summary.is_empty());
        rig.gate
            .call(method, params.clone(), Some(&token))
            .await
            .unwrap();
        assert_eq!(
            rig.transport.frames(),
            vec![Frame {
                method: method.to_owned(),
                params
            }]
        );
    }
}

/// PREDICT: a declined dialog yields no token and an error; nothing is sent.
#[tokio::test]
async fn declined_confirmation_issues_no_token() {
    let rig = rig();
    rig.confirmer.then(Confirmation::Declined);
    let (method, params) = &p2_cases()[0];
    let r = rig.gate.request_confirmation(method, params).await;
    assert!(
        matches!(r, Err(GateError::ConfirmationDeclined { .. })),
        "{r:?}"
    );
    assert!(rig.transport.frames().is_empty());
}

/// PREDICT: a token for other params is refused (Mismatch) and sends nothing; it is not burned,
/// so the call it was issued for still works afterwards.
#[tokio::test]
async fn token_for_other_params_is_refused_and_not_burned() {
    let rig = rig();
    let good = json!({"sessionId": "s1", "agentName": "helper"});
    let evil = json!({"sessionId": "s1", "agentName": "evil"});
    let token = rig
        .gate
        .request_confirmation("agents/install", &good)
        .await
        .unwrap();
    let r = rig.gate.call("agents/install", evil, Some(&token)).await;
    assert!(
        matches!(
            r,
            Err(GateError::ConfirmationRejected {
                fault: TokenFault::Mismatch,
                ..
            })
        ),
        "{r:?}"
    );
    assert!(rig.transport.frames().is_empty());
    rig.gate
        .call("agents/install", good, Some(&token))
        .await
        .unwrap();
    assert_eq!(rig.transport.frames().len(), 1);
}

/// PREDICT: a token issued for one method does not work for another method, even with the same
/// params (agents/install token must not unlock agents/uninstall).
#[tokio::test]
async fn token_for_other_method_is_refused() {
    let rig = rig();
    let params = json!({"sessionId": "s1", "agentName": "helper"});
    let token = rig
        .gate
        .request_confirmation("agents/install", &params)
        .await
        .unwrap();
    let r = rig
        .gate
        .call("agents/uninstall", params, Some(&token))
        .await;
    assert!(
        matches!(
            r,
            Err(GateError::ConfirmationRejected {
                fault: TokenFault::Mismatch,
                ..
            })
        ),
        "{r:?}"
    );
    assert!(rig.transport.frames().is_empty());
}

/// PREDICT: a confirmed call the server refuses for a moment is sent once more with the same spent token (two
/// frames, the second answer returned), and the token cannot be used again afterwards; an error the caller does not
/// accept is not sent again.
#[tokio::test]
async fn a_passing_refusal_resends_the_confirmed_call_once() {
    let rig = rig();
    let (method, params) = &p2_cases()[2];
    let passing = |error: &GateError| format!("{error}").contains("in use");
    let token = rig.gate.request_confirmation(method, params).await.unwrap();
    rig.transport.fail_next(method, "the file is in use");
    let r = rig
        .gate
        .call_with_retry(method, params.clone(), Some(&token), passing, || async {})
        .await;
    assert!(r.is_ok(), "{r:?}");
    assert_eq!(rig.transport.frames().len(), 2);
    let again = rig.gate.call(method, params.clone(), Some(&token)).await;
    assert!(
        matches!(
            again,
            Err(GateError::ConfirmationRejected {
                fault: TokenFault::Reused,
                ..
            })
        ),
        "{again:?}"
    );

    let token = rig.gate.request_confirmation(method, params).await.unwrap();
    rig.transport.fail_next(method, "not found");
    let r = rig
        .gate
        .call_with_retry(method, params.clone(), Some(&token), passing, || async {})
        .await;
    assert!(matches!(r, Err(GateError::Transport(_))), "{r:?}");
    assert_eq!(rig.transport.frames().len(), 3);
}

/// PREDICT: reuse is refused with its own fault, and the second send never happens.
#[tokio::test]
async fn token_is_single_use() {
    let rig = rig();
    let (method, params) = &p2_cases()[2];
    let token = rig.gate.request_confirmation(method, params).await.unwrap();
    rig.gate
        .call(method, params.clone(), Some(&token))
        .await
        .unwrap();
    let r = rig.gate.call(method, params.clone(), Some(&token)).await;
    assert!(
        matches!(
            r,
            Err(GateError::ConfirmationRejected {
                fault: TokenFault::Reused,
                ..
            })
        ),
        "{r:?}"
    );
    assert_eq!(rig.transport.frames().len(), 1);
}

/// PREDICT: a token is valid for the configured lifetime and not a moment longer.
#[tokio::test]
async fn token_expires() {
    let rig = rig_with(|c| c.token_ttl = Duration::from_secs(30));
    let (method, params) = &p2_cases()[2];
    let live = rig.gate.request_confirmation(method, params).await.unwrap();
    rig.clock.advance(Duration::from_secs(29));
    rig.gate
        .call(method, params.clone(), Some(&live))
        .await
        .unwrap();

    let stale = rig.gate.request_confirmation(method, params).await.unwrap();
    rig.clock.advance(Duration::from_secs(31));
    let r = rig.gate.call(method, params.clone(), Some(&stale)).await;
    assert!(
        matches!(
            r,
            Err(GateError::ConfirmationRejected {
                fault: TokenFault::Expired,
                ..
            })
        ),
        "{r:?}"
    );
    assert_eq!(
        rig.transport.frames().len(),
        1,
        "only the live token's call was sent"
    );
}

/// PREDICT: invented, empty and altered tokens are refused as Unknown.
#[tokio::test]
async fn forged_tokens_are_refused() {
    let rig = rig();
    let (method, params) = &p2_cases()[2];
    let real = rig.gate.request_confirmation(method, params).await.unwrap();
    let altered = format!("{}0", real.as_str());
    let flipped: String = real.as_str().chars().rev().collect();
    for forged in [
        "",
        "token",
        "00000000000000000000000000000000",
        &altered,
        &flipped,
    ] {
        let t = ConfirmationToken::from_wire(forged.to_owned());
        let r = rig.gate.call(method, params.clone(), Some(&t)).await;
        assert!(
            matches!(
                r,
                Err(GateError::ConfirmationRejected {
                    fault: TokenFault::Unknown,
                    ..
                })
            ),
            "{forged:?}: {r:?}"
        );
    }
    assert!(rig.transport.frames().is_empty());
}

/// PREDICT: the token binds to the params' meaning, not their key order.
#[tokio::test]
async fn token_ignores_object_key_order() {
    let rig = rig();
    let a: Value = serde_json::from_str(r#"{"sessionId":"s1","agentName":"helper"}"#).unwrap();
    let b: Value = serde_json::from_str(r#"{"agentName":"helper","sessionId":"s1"}"#).unwrap();
    let token = rig
        .gate
        .request_confirmation("agents/install", &a)
        .await
        .unwrap();
    rig.gate
        .call("agents/install", b, Some(&token))
        .await
        .unwrap();
    assert_eq!(rig.transport.frames().len(), 1);
}

/// PREDICT: no confirmation can be requested for a forwarded or unknown method (a token must not
/// be mintable for something the gate would allow or deny on its own); malformed params are
/// refused before the user is asked anything.
#[tokio::test]
async fn confirmation_is_only_offered_for_confirmable_calls() {
    let rig = rig();
    for method in [
        "session/list",
        "setup/store-credential",
        "nonsense/method",
        "callback/result",
    ] {
        let r = rig.gate.request_confirmation(method, &json!({})).await;
        assert!(r.is_err(), "{method}: {r:?}");
    }
    let r = rig
        .gate
        .request_confirmation("agents/install", &json!({"agentName": 5}))
        .await;
    assert!(matches!(r, Err(GateError::InvalidParams { .. })), "{r:?}");
    assert!(
        rig.confirmer.shown().is_empty(),
        "the user was asked about a request that could never pass"
    );
}

/// PREDICT: the number of live tokens is bounded; the oldest is dropped when the cap is hit.
#[tokio::test]
async fn token_count_is_bounded() {
    let rig = rig_with(|c| c.max_tokens = 2);
    let (method, _) = &p2_cases()[2];
    let mut tokens = Vec::new();
    for i in 0..3 {
        let params = json!({"sessionId": "s1", "agentName": format!("a{i}")});
        let t = rig
            .gate
            .request_confirmation(method, &params)
            .await
            .unwrap();
        tokens.push((t, params));
    }
    let (t0, p0) = &tokens[0];
    let r = rig.gate.call(method, p0.clone(), Some(t0)).await;
    assert!(
        matches!(r, Err(GateError::ConfirmationRejected { .. })),
        "oldest token should be gone: {r:?}"
    );
    let (t2, p2) = &tokens[2];
    rig.gate.call(method, p2.clone(), Some(t2)).await.unwrap();
}

/// PREDICT: escalating shapes of otherwise-forwarded or typed calls need a token too: the
/// auto-approve profile, `autoApprove: true`, a non-inspect worktree removal with explicit params, and
/// trusting the whole repository. The same calls in their safe shape need none. A worktree removal on
/// unspecified options, `prune` and `reap` are refused outright.
#[tokio::test]
async fn escalations_need_a_token_and_safe_shapes_do_not() {
    let rig = rig();
    let needs: Vec<(&str, Value)> = vec![
        (
            "session/agent/update",
            json!({"sessionId": "s1", "agentName": "auto-approve"}),
        ),
        (
            "session/start",
            json!({"agentConfig": {"cwd": "/work/r", "autoApprove": true}}),
        ),
        (
            "session/start",
            json!({"agentConfig": {"cwd": "/work/r", "agent": "auto-approve"}}),
        ),
        (
            "workspace/git/worktrees/remove",
            json!({"cwd": "/work/wt", "force": true, "deleteBranch": false, "inspect": false}),
        ),
        (
            "workspace/git/worktrees/remove",
            json!({"cwd": "/work/wt", "force": false, "deleteBranch": true, "inspect": false}),
        ),
    ];
    for (method, params) in &needs {
        assert_required(rig.gate.call(method, params.clone(), None).await, method);
    }
    // a removal that leaves any of the three unspecified is refused
    // outright, also when a confirmation would be given.
    let defaults = [
        json!({"cwd": "/work/wt"}),
        json!({"cwd": "/work/wt", "force": true}),
        json!({"cwd": "/work/wt", "inspect": false}),
        json!({"cwd": "/work/wt", "force": true, "deleteBranch": true}),
        json!({"cwd": "/work/wt", "force": true, "inspect": true}),
    ];
    for params in defaults {
        let r = rig
            .gate
            .call("workspace/git/worktrees/remove", params.clone(), None)
            .await;
        assert!(
            matches!(&r, Err(GateError::Denied { .. })),
            "{params}: {r:?}"
        );
        assert!(
            rig.gate
                .request_confirmation("workspace/git/worktrees/remove", &params)
                .await
                .is_err(),
            "{params}: a dialog was offered for a default removal"
        );
    }
    for method in [
        "workspace/git/worktrees/prune",
        "workspace/git/worktrees/reap",
    ] {
        let r = rig
            .gate
            .call(method, json!({"cwd": "/work/wt"}), None)
            .await;
        assert!(
            matches!(&r, Err(GateError::Denied { .. })),
            "{method}: {r:?}"
        );
    }
    assert!(rig.transport.frames().is_empty());
    // With a token the escalated call goes through, unchanged apart from the documented defaults.
    let params = json!({"sessionId": "s1", "agentName": "auto-approve"});
    let t = rig
        .gate
        .request_confirmation("session/agent/update", &params)
        .await
        .unwrap();
    rig.gate
        .call("session/agent/update", params.clone(), Some(&t))
        .await
        .unwrap();
    assert_eq!(
        rig.transport.frames(),
        vec![Frame {
            method: "session/agent/update".into(),
            params
        }]
    );

    let safe: Vec<(&str, Value)> = vec![
        (
            "session/agent/update",
            json!({"sessionId": "s1", "agentName": "ask"}),
        ),
        (
            "workspace/git/worktrees/remove",
            json!({"cwd": "/work/wt", "inspect": true}),
        ),
    ];
    for (method, params) in safe {
        rig.gate.call(method, params, None).await.unwrap();
    }
}
