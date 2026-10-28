//! First use of the user's Chrome in a chat: the one `AskCard`, session Allow / No, with
//! audit record. Depends on vw-ask, on-demand session state and the Chrome card publisher / outcome ledger.
//! Called by the pre-tool hook. The call waits for the person; No and Stop cannot enable Chrome.
use std::sync::Arc;

use serde_json::json;
use vw_ask::{Choice, CoversPart, Outcome};

use super::ChromeAskView;
use crate::core::Shared;
use crate::on_demand::OnDemand;

/// Asks before Chrome is first made available in this chat. Only Allow for this session remembers consent.
pub(crate) async fn ensure(
    shared: &Arc<Shared>,
    state: &OnDemand,
    session: &str,
    source: &str,
) -> Result<(), String> {
    let generation = shared
        .chrome_control
        .generation
        .load(std::sync::atomic::Ordering::SeqCst);
    let wait = state.chrome_wait(session);
    let _held = wait.lock().await;
    if state.chrome_allowed(session) {
        return Ok(());
    }
    let outcome = super::pending::answer(shared, session, || {
        let handle = shared.asks.open_own(
            vw_ask::chrome::SESSION_KIND,
            Some(source.to_owned()),
            vw_ask::Requester::Agent,
            vec![Choice::Session, Choice::No],
            None,
        );
        let summary = "Vibe wants to use your Chrome in this chat.";
        let card = ChromeAskView {
            ask_id: handle.ask.id.clone(),
            session_id: session.to_owned(),
            turn_id: shared.store.view(session).and_then(|view| {
                view.state
                    .pointer("/session/status/activeTurnId")
                    .and_then(|v| v.as_str())
                    .map(str::to_owned)
            }),
            created_at_ms: shared.asks.now_ms(),
            tool: "mcp_vw.use_your_chrome".to_owned(),
            summary: summary.to_owned(),
            group: None,
            reason: String::new(),
            covers: vec![CoversPart {
                text: "using your Chrome in this chat".to_owned(),
                code: false,
            }],
            input: json!({}),
            open: true,
        };
        (handle, card)
    })
    .await?;
    if matches!(
        outcome,
        Outcome::Allowed {
            scope: vw_ask::Scope::Session,
            ..
        }
    ) {
        if generation
            != shared
                .chrome_control
                .generation
                .load(std::sync::atomic::Ordering::SeqCst)
        {
            return Err("Not run: Chrome Control was removed.".to_owned());
        }
        state.allow_chrome(session);
        return Ok(());
    }
    let message = vw_ask::agent_text(&outcome, "using your Chrome in this chat");
    Err(message)
}
