//! User-only provider sign-in pages and cookie-presence hints for Settings.
//!
//! Depends on: preview service and the core command API. Called by: browser command handler.
//! Invariants: fixed public sign-in URLs, no session or agent tooling for this page, no cookie values in answers.

use std::sync::Arc;

use vw_preview::{OpenRequest, accounts::Provider};

use crate::browser_view::{BrowserRequest, BrowserResult};
use crate::core::Shared;
use crate::error::{CoreError, ErrorCode};
use crate::previews::hub;

/// A reserved browser entry that is never a loaded agent session.
pub(crate) const ACCOUNT_PAGE: &str = "\0signed-in-accounts";

fn failure(error: impl std::fmt::Display) -> CoreError {
    CoreError::new(ErrorCode::Denied, error.to_string())
}

/// Reports a wiring error if an account command escaped its dedicated handler.
pub(crate) fn unhandled() -> CoreError {
    CoreError::new(ErrorCode::Internal, "account request was not handled")
}

/// Refuses tooling access to the reserved page where the user signs in by hand.
pub(crate) fn admit(request: &BrowserRequest) -> Result<(), CoreError> {
    let session = match request {
        BrowserRequest::Tabs { session_id }
        | BrowserRequest::OpenTab { session_id, .. }
        | BrowserRequest::SwitchTab { session_id, .. }
        | BrowserRequest::CloseTab { session_id, .. }
        | BrowserRequest::ReorderTabs { session_id, .. }
        | BrowserRequest::ReviewHandoff { session_id, .. }
        | BrowserRequest::HandoffTab { session_id, .. }
        | BrowserRequest::TakeBackTab { session_id, .. }
        | BrowserRequest::Read { session_id, .. }
        | BrowserRequest::Act { session_id, .. }
        | BrowserRequest::State { session_id }
        | BrowserRequest::Stop { session_id }
        | BrowserRequest::Resume { session_id }
        | BrowserRequest::Annotate { session_id, .. }
        | BrowserRequest::CaptureSecret { session_id, .. }
        | BrowserRequest::ChooseSecret { session_id, .. }
        | BrowserRequest::SaveSecret { session_id, .. }
        | BrowserRequest::CancelSecret { session_id, .. }
        | BrowserRequest::Capture { session_id, .. }
        | BrowserRequest::CancelAnnotate { session_id }
        | BrowserRequest::CheckAnnotations { session_id, .. } => Some(session_id.as_str()),
        _ => None,
    };
    if session == Some(ACCOUNT_PAGE) {
        return Err(failure("this sign-in page is for the user only"));
    }
    Ok(())
}

async fn accounts(shared: &Shared) -> Result<BrowserResult, CoreError> {
    Ok(BrowserResult::Accounts {
        accounts: hub(shared)?.service.accounts().await.map_err(failure)?,
    })
}

async fn sign_in(shared: &Shared, provider: Provider) -> Result<BrowserResult, CoreError> {
    let service = &hub(shared)?.service;
    let url = url::Url::parse(provider.sign_in_url()).map_err(failure)?;
    service
        .open(OpenRequest {
            session_id: ACCOUNT_PAGE.to_owned(),
            url,
            allowed_ports: Vec::new(),
            downloads_dir: None,
            wait: false,
            lan: false,
        })
        .await
        .map_err(failure)?;
    Ok(BrowserResult::Done {})
}

/// Handles account requests; other browser requests are returned to their usual handler.
pub(crate) async fn run(
    shared: &Arc<Shared>,
    request: &BrowserRequest,
) -> Option<Result<BrowserResult, CoreError>> {
    let service = match hub(shared) {
        Ok(hub) => &hub.service,
        Err(error) => return Some(Err(error)),
    };
    Some(match request {
        BrowserRequest::Accounts {} => accounts(shared).await,
        BrowserRequest::SignIn { provider } => sign_in(shared, *provider).await,
        BrowserRequest::SignOut { provider } => match service.sign_out(*provider).await {
            Ok(()) => accounts(shared).await,
            Err(error) => Err(failure(error)),
        },
        BrowserRequest::AccountsLayout { rect } => service
            .layout(rect.map(|rect| (ACCOUNT_PAGE.to_owned(), rect)))
            .await
            .map(|()| BrowserResult::Done {})
            .map_err(failure),
        BrowserRequest::CloseSignIn {} => service
            .close(ACCOUNT_PAGE)
            .await
            .map(|()| BrowserResult::Done {})
            .map_err(failure),
        _ => return None,
    })
}
