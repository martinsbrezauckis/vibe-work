//! Session deletion approval through the core and real stored Vibe sessions.
//! PREDICT: a user decline sends nothing, approval sends exactly once, and reuse is refused.

mod common;

use common::core::{ScriptRig, stored_core_with};
use serde_json::{Value, json};
use vw_core_api::{Command, CommandResult, CoreError, ErrorCode, Origin};
use vw_gate::Confirmation;

async fn rig(name: &str) -> ScriptRig {
    let items: Vec<Value> = (1..=5)
        .map(|n| {
            json!({
                "id":format!("old-{n}"),"title":format!("Old session {n}"),"status":{"type":"idle"},
                "updatedAt":1000-n,"cwd":"/work/alpha","isUnseen":n==2,
                "archivedAt":if n==5 {json!(7)} else {Value::Null}
            })
        })
        .collect();
    stored_core_with(name, None, &items, common::real_vibe::text_script(), |_| {}).await
}

async fn list(
    rig: &ScriptRig,
    limit: Option<u32>,
    cursor: Option<String>,
    archived: Option<bool>,
) -> (Vec<Value>, Option<String>, Option<String>) {
    let command = Command::ListSessions {
        limit,
        cursor,
        include_archived: archived,
    };
    match rig.core.execute(Origin::Ui, command).await.unwrap() {
        CommandResult::Sessions {
            rows,
            next_cursor,
            continue_session_id,
        } => (
            rows.into_iter().map(|r| r.session).collect(),
            next_cursor,
            continue_session_id,
        ),
        other => panic!("{other:?}"),
    }
}

fn ids(rows: &[Value]) -> Vec<String> {
    rows.iter()
        .map(|r| r["id"].as_str().unwrap().to_owned())
        .collect()
}

/// The window's delete needs the native dialog naming the session; the Debug API deletes directly.
#[tokio::test]
async fn delete_needs_the_native_dialog_and_names_the_session() {
    let rig = rig("idx-delete").await;
    list(&rig, None, None, None).await; // the core learns titles and folders from the listing
    let params = json!({"sessionId": rig.id("old-2")});
    let call = |token: Option<String>| Command::Call {
        method: "session/delete".to_owned(),
        params: params.clone(),
        confirmation_token: token,
    };

    // The window, no token: refused, nothing reaches the server.
    let err = rig.core.execute(Origin::Ui, call(None)).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::ConfirmationRequired);
    assert!(!rig.received().contains(&"session/delete".to_owned()));

    // The dialog is declined: still nothing sent, and no token exists.
    rig.confirmer.then(Confirmation::Declined);
    let ask = || Command::RequestConfirmation {
        method: "session/delete".to_owned(),
        params: params.clone(),
    };
    let err = rig.core.execute(Origin::Ui, ask()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::ConfirmationDeclined);
    assert!(!rig.received().contains(&"session/delete".to_owned()));
    let shown = rig.confirmer.shown();
    assert!(
        shown[0].summary.contains("Old session 2"),
        "the dialog names the session: {}",
        shown[0].summary
    );

    // Approved at the dialog: a token, and the delete reaches the server once.
    rig.confirmer.then(Confirmation::Approved);
    let CommandResult::Confirmation { token } = rig.core.execute(Origin::Ui, ask()).await.unwrap()
    else {
        panic!()
    };
    rig.core
        .execute(Origin::Ui, call(Some(token.clone())))
        .await
        .unwrap();
    let sent = |rig: &ScriptRig| {
        rig.received()
            .iter()
            .filter(|m| *m == "session/delete")
            .count()
    };
    assert_eq!(sent(&rig), 1);
    let again: Result<_, CoreError> = rig.core.execute(Origin::Ui, call(Some(token))).await;
    assert!(again.is_err(), "the token is single use");
    let (rows, _, _) = list(&rig, None, None, None).await;
    assert!(!ids(&rows).contains(&rig.id("old-2")));

    // The Debug API: no token, no dialog (a confirmer scripted to decline is not consulted).
    let shown_before = rig.confirmer.shown().len();
    rig.confirmer.then(Confirmation::Declined);
    rig.core
        .execute(
            Origin::DebugApi,
            Command::Call {
                method: "session/delete".to_owned(),
                params: json!({"sessionId": rig.id("old-3")}),
                confirmation_token: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(sent(&rig), 2);
    assert_eq!(rig.confirmer.shown().len(), shown_before);
    rig.core.shutdown().await;
}
