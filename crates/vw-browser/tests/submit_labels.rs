//! Visible labels for hidden submit controls remain discoverable and retain ordinary send approval.
//! Uses the production browser driver in a disposable Chromium document; no form reaches the network.
#![cfg(feature = "e2e-script")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;
use serde_json::json;
use support::{Rig, SESSION};
use vw_browser::{Actor, ReadOptions};

struct NoNavigation;
impl vw_browser::Navigate for NoNavigation {
    fn open<'a>(&'a self, _: &'a str, _: &'a str) -> vw_browser::NavFuture<'a> {
        Box::pin(async { Err("unexpected navigation".into()) })
    }
}

#[tokio::test]
async fn visible_submit_labels_have_their_own_row_and_require_send_approval() {
    let rig = Rig::new().await;
    for control in [
        "<button id=send hidden>Submit</button>",
        "<input id=send type=submit style='display:none' value=Submit>",
        "<input id=send type=image hidden alt=Submit>",
    ] {
        for wrapping in [false, true] {
            let contents = if wrapping {
                format!("<label id=label>Send order{control}</label>")
            } else {
                format!("<label id=label for=send>Send order</label>{control}")
            };
            rig.engine.eval(&format!(
                "document.body.innerHTML={}; window.sends=0; document.querySelector('form').onsubmit=e=>{{e.preventDefault();sends++}}; scrollTo(0,0)",
                json!(format!("<form method=post>{contents}</form>"))
            )).await;
            let (model, _, _) = rig
                .browser
                .observe(SESSION, ReadOptions::default())
                .await
                .unwrap();
            let label = model
                .refs
                .iter()
                .find(|row| row.name == "Send order" && row.visible != Some(false));
            assert!(label.is_some(), "missing visible submit label: {model:?}");
            let label = label.unwrap();
            assert_eq!(label.role, "button");
            let action = vw_browser::parse(&json!({"action":"click","ref":label.id})).unwrap();
            let denied = rig
                .browser
                .act_with_sending(SESSION, action.clone(), Actor::Agent, &NoNavigation, false)
                .await
                .unwrap_err();
            assert_eq!(denied.text, vw_browser::SEND_REQUIRED);
            assert_eq!(rig.engine.eval("sends").await, 0);
            rig.browser
                .act_with_sending(SESSION, action, Actor::Agent, &NoNavigation, true)
                .await
                .unwrap();
            assert_eq!(rig.engine.eval("sends").await, 1);
        }
    }
}
