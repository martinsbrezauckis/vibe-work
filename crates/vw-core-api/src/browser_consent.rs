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

/// A native-only confirmation method for wider provider permissions.
pub(crate) const METHOD: &str = "vibe-work/providerConsent";

fn describe_scope(scope: &str) -> String {
    let lower = scope.to_ascii_lowercase();
    let label = if matches!(
        lower.as_str(),
        "openid" | "profile" | "name" | "read:user" | "user.read"
    ) || lower.ends_with("/userinfo.profile")
    {
        "Read your basic profile"
    } else if matches!(lower.as_str(), "email" | "user:email") || lower.ends_with("/userinfo.email")
    {
        "Read your email address"
    } else if lower.contains("offline") {
        "Keep access when you are away"
    } else if lower.contains("mail") || lower.contains("gmail") {
        if lower.contains("read") {
            "Read your mail"
        } else {
            "Read or change your mail"
        }
    } else if lower.contains("drive") || lower.contains("files") {
        if lower.contains("read") {
            "Read your files"
        } else {
            "Read or change your files"
        }
    } else if lower.contains("calendar") {
        if lower.contains("read") {
            "Read your calendar"
        } else {
            "Read or change your calendar"
        }
    } else if lower.contains("contacts") {
        if lower.contains("write") {
            "Read or change your contacts"
        } else {
            "Read your contacts"
        }
    } else if lower.contains("repo") || lower.contains("contents") {
        if lower.contains("read") {
            "Read your repositories"
        } else {
            "Read or change your repositories"
        }
    } else {
        "Unrecognized permission"
    };
    format!("{label} ({})", vw_gate::dialog_quote(scope))
}

struct NativeConsent(Weak<Shared>);

impl ConsentConfirmer for NativeConsent {
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
                "Allow service {} (client {}, shown on page {}) to access your {} account with these permissions: {}?\n\nSession: {} (ID {})",
                vw_gate::dialog_quote(&service),
                vw_gate::dialog_quote(&request.client),
                vw_gate::dialog_quote(&request.title),
                request.provider.name(),
                scopes,
                vw_gate::dialog_quote(
                    &shared
                        .names
                        .title_of(&shared.store, &owner)
                        .unwrap_or_else(|| owner.clone()),
                ),
                vw_gate::dialog_quote(&owner),
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
            shared.confirmer.confirm(&confirmation).await.approved()
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
