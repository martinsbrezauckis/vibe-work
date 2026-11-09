//! Native confirmation for automated wider provider authorization in the built-in browser.
//!
//! Depends on: `vw-browser` consent callback, the core's native confirmer, and the preview hub. Called by: core
//! startup. Invariant: the dialog is for the user and the agent; a Debug API action is answered by the core's
//! confirmer like any other confirmation (`confirmer.rs`), because the Debug API acts as the user.

use std::sync::{Arc, Weak};

use serde_json::json;
use vw_browser::{
    Actor,
    consent::{ConsentConfirmer, ConsentFuture, ConsentRequest},
};
use vw_gate::{ConfirmationRequest, Confirmer, Requester};

use crate::core::Shared;

#[cfg(test)]
#[path = "browser_consent_tests.rs"]
mod tests;

/// A native-only confirmation method for wider provider permissions.
pub(crate) const METHOD: &str = "vibe-work/providerConsent";

fn describe_scope(scope: &str) -> String {
    // Scope names are exact tokens, not prose: a familiar substring cannot establish read-only access.
    let label = match scope {
        "openid"
        | "profile"
        | "name"
        | "read:user"
        | "User.Read"
        | "https://www.googleapis.com/auth/userinfo.profile" => "Read your basic profile",
        "email" | "user:email" | "https://www.googleapis.com/auth/userinfo.email" => {
            "Read your email address"
        }
        "offline_access" => "Keep access when you are away",
        "Mail.Read" | "https://www.googleapis.com/auth/gmail.readonly" => "Read your mail",
        "Mail.ReadWrite" => "Read or change your mail",
        "Mail.Send" | "https://www.googleapis.com/auth/gmail.send" => "Send mail",
        "https://www.googleapis.com/auth/gmail.compose" => "Read or change drafts and send mail",
        "https://www.googleapis.com/auth/gmail.modify" => "Read or change your mail and send mail",
        "https://mail.google.com/" => "Read, change, send or permanently delete all your mail",
        "Files.Read" | "Files.Read.All" | "https://www.googleapis.com/auth/drive.readonly" => {
            "Read your files"
        }
        "Files.ReadWrite"
        | "Files.ReadWrite.All"
        | "https://www.googleapis.com/auth/drive"
        | "https://www.googleapis.com/auth/drive.file" => "Read or change your files",
        "Calendars.Read"
        | "https://www.googleapis.com/auth/calendar.readonly"
        | "https://www.googleapis.com/auth/calendar.events.readonly" => "Read your calendar",
        "Calendars.ReadWrite"
        | "https://www.googleapis.com/auth/calendar"
        | "https://www.googleapis.com/auth/calendar.events" => "Read or change your calendar",
        "Contacts.Read" | "https://www.googleapis.com/auth/contacts.readonly" => {
            "Read your contacts"
        }
        "Contacts.ReadWrite" | "https://www.googleapis.com/auth/contacts" => {
            "Read or change your contacts"
        }
        "repo" => {
            "Manage repositories and organization projects, invitations, team memberships and webhooks"
        }
        "public_repo" => "Read or change your repositories",
        _ => "Unrecognized permission",
    };
    format!("{label} ({})", vw_gate::dialog_quote(scope))
}

struct NativeConsent(Weak<Shared>);

impl ConsentConfirmer for NativeConsent {
    fn refusal_text(&self, session_id: &str) -> Option<String> {
        let shared = self.0.upgrade()?;
        let owner = shared
            .preview
            .as_ref()
            .and_then(|hub| hub.service.owner_session_id(session_id))
            .unwrap_or_else(|| session_id.into());
        let record =
            shared.asks.records().into_iter().rev().find(|r| {
                r.ask.kind == METHOD && r.ask.session.as_deref() == Some(owner.as_str())
            })?;
        Some(vw_ask::agent_text(&record.outcome, &record.ask.title))
    }

    fn confirm(&self, request: ConsentRequest) -> ConsentFuture<'_> {
        Box::pin(async move {
            let Some(shared) = self.0.upgrade() else {
                return false;
            };
            let Some(owner) = shared
                .preview
                .as_ref()
                .and_then(|hub| hub.service.owner_session_id(&request.session_id))
            else {
                return false;
            };
            if shared.store.view(&owner).is_none() {
                return false;
            }
            let requester = match request.actor {
                Actor::Agent => Requester::Agent,
                Actor::DebugApi => Requester::DebugApi,
                Actor::User => return false,
            };
            let service = if request.title.trim().is_empty() && request.client.trim().is_empty() {
                "Unknown service".to_owned()
            } else if request.title.trim().is_empty() {
                request.client.clone()
            } else if request.client.trim().is_empty() || request.title == request.client {
                request.title.clone()
            } else {
                format!("{} (client {})", request.title, request.client)
            };
            let mut scopes = vw_browser::consent::scope_tokens(&request.scopes)
                .iter()
                .map(|scope| describe_scope(scope))
                .collect::<Vec<_>>()
                .join("; ");
            if scopes.is_empty() {
                "Permissions were not supplied; the requested access cannot be verified"
                    .clone_into(&mut scopes);
            }
            let summary = format!(
                "Allow service {} (client {}, shown on page {}) to access your {} account with these permissions: {}?\n\nSession: {}",
                vw_gate::dialog_quote(&service),
                vw_gate::dialog_quote(&request.client),
                vw_gate::dialog_quote(&request.title),
                request.provider.name(),
                scopes,
                vw_gate::dialog_chat_quote(&shared.names.confirmation_title(&shared.store, &owner),),
            );
            let confirmation = ConfirmationRequest {
                method: METHOD.to_owned(),
                summary,
                params: json!({
                    "sessionId": owner,
                    "provider": request.provider.name(),
                    "service": service,
                    "clientId": request.client,
                    "pageTitle": request.title,
                    "scopes": request.scopes,
                }),
                requester,
                always: None,
            };
            if requester == Requester::Agent {
                crate::ask::guards::wait(&shared, confirmation, "this provider permission")
                    .await
                    .is_ok()
            } else {
                shared.confirmer.confirm(&confirmation).await.approved()
            }
        })
    }
}

/// Connects the preview browser to the core's native confirmation service.
pub(crate) fn install(shared: &Arc<Shared>) {
    if let Some(hub) = &shared.preview {
        hub.browser
            .set_consent_confirmer(Arc::new(NativeConsent(Arc::downgrade(shared))));
    }
}
