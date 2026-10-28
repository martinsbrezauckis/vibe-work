//! Native confirmation adapter for the gate and core features.
//! Depends on the shared ask service, native presenter, session names and exact-call dialog text registry.
//! Core construction installs it once; every native feature and gated confirmation uses the same adapter.
//!
//! Every request opens a value-free ask before presentation. The service records the first answer or cancellation; dropping the presentation ends its permission wait and makes late answers ineffective.
//! The Debug API acts as the user outside the vault, and Core may complete an earlier user choice without
//! another dialog. Vault policy always requires the person's native answer, regardless of requester.
//! Dialogs retain their complete feature explanations and existing offered scopes. Records use policy labels,
//! never request parameters or dialog bodies. An unoffered lasting answer is reduced to Allow once.

use std::collections::BTreeSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};

use vw_gate::{Confirmation, ConfirmationRequest, Confirmer, Requester};
use vw_session::SessionStore;

use crate::dialog_texts::DialogTexts;
use crate::names::Names;

type Asking = Pin<Box<dyn Future<Output = Confirmation> + Send>>;

/// Holds any `Confirmer` behind one function pointer, and the sentences the core registered for exact calls.
#[derive(Clone)]
pub(crate) struct DynConfirmer(
    Arc<dyn Fn(ConfirmationRequest) -> Asking + Send + Sync>,
    Arc<DialogTexts>,
    Arc<Mutex<BTreeSet<String>>>,
);

impl DynConfirmer {
    /// `names` and `store` let the dialog text name a session by its title instead of its id. Sentences the
    /// core writes for one exact call (a worktree removal naming its files) go through
    /// [`DynConfirmer::texts`].
    pub(crate) fn new<C: Confirmer + 'static>(
        confirmer: C,
        names: Arc<Names>,
        store: SessionStore,
        asks: Arc<crate::ask::Service>,
    ) -> Self {
        let confirmer = Arc::new(confirmer);
        let texts = Arc::new(DialogTexts::default());
        let registry = texts.clone();
        let deletes = Arc::new(Mutex::new(BTreeSet::new()));
        let approved_deletes = deletes.clone();
        let ask = Arc::new(move |mut request: ConfirmationRequest| {
            let deletes = approved_deletes.clone();
            let by = match request.requester {
                Requester::DebugApi => vw_ask::AnsweredBy::DebugApi,
                Requester::Core => vw_ask::AnsweredBy::Core,
                Requester::Window | Requester::Agent => vw_ask::AnsweredBy::Person,
            };
            let session = request
                .params
                .get("sessionId")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned);
            // `askId` names the guard's or secret run's own id; `open_own` never joins another ask with it.
            let mut handle = asks.open_own(
                &request.method,
                session,
                match request.requester {
                    Requester::Agent => vw_ask::Requester::Agent,
                    Requester::Window => vw_ask::Requester::Window,
                    Requester::DebugApi => vw_ask::Requester::DebugApi,
                    Requester::Core => vw_ask::Requester::Core,
                },
                vec![vw_ask::Choice::Once, vw_ask::Choice::No],
                request
                    .params
                    .get("askId")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned),
            );
            handle.cancel_on_drop = true;
            let automatic = matches!(request.requester, Requester::Core | Requester::DebugApi)
                && !handle.ask.person_only;
            if automatic {
                handle.finish(crate::ask::allowed(by, vw_ask::Scope::Once));
                if request.requester == Requester::DebugApi {
                    record_delete(&deletes, &request);
                }
                return Box::pin(std::future::ready(Confirmation::Approved)) as Asking;
            }
            let confirmer = confirmer.clone();
            decorate(&mut request, &texts, &names, &store);
            // The presenter lists a window of its own under the ask's id, so the window can bring it to the front
            // (after `decorate`: the registered sentences are found by the request's own params).
            if let Some(fields) = request.params.as_object_mut() {
                fields.insert("askId".to_owned(), handle.ask.id.clone().into());
            }
            Box::pin(async move {
                let offered = request.always.is_some();
                let confirmation = tokio::select! {
                    biased;
                    outcome = handle.closed() => return as_confirmation(&outcome),
                    answer = confirmer.confirm(&request) => answer,
                };
                let confirmation = match confirmation {
                    Confirmation::ApprovedAlways if !offered => Confirmation::Approved,
                    answer => answer,
                };
                let outcome = match confirmation {
                    Confirmation::Approved => {
                        crate::ask::allowed(vw_ask::AnsweredBy::Person, vw_ask::Scope::Once)
                    }
                    Confirmation::ApprovedAlways => {
                        crate::ask::allowed(vw_ask::AnsweredBy::Person, vw_ask::Scope::Level)
                    }
                    Confirmation::Declined => crate::ask::refused(vw_ask::AnsweredBy::Person, None),
                };
                handle.finish(outcome);
                let answer = as_confirmation(&handle.closed().await);
                if matches!(
                    answer,
                    Confirmation::Approved | Confirmation::ApprovedAlways
                ) {
                    record_delete(&deletes, &request);
                }
                answer
            }) as Asking
        });
        Self(ask, registry, deletes)
    }

    /// A person (or the Debug API acting as the person) approved Delete of this exact id.
    pub(crate) fn deletion_confirmed(&self, id: &str) -> bool {
        self.2
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(id)
    }

    pub(crate) fn deletion_finished(&self, id: &str) {
        self.2
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(id);
    }

    /// Registers the native ask before its owned presentation is scheduled.
    pub(crate) fn begin(&self, request: ConfirmationRequest) -> Asking {
        (self.0)(request)
    }

    /// The registry of sentences for exact calls (shared with the core).
    pub(crate) fn texts(&self) -> Arc<DialogTexts> {
        self.1.clone()
    }
}

fn record_delete(deletes: &Mutex<BTreeSet<String>>, request: &ConfirmationRequest) {
    if request.method == "session/delete"
        && let Some(id) = request.params["sessionId"].as_str()
    {
        deletes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id.to_owned());
    }
}

impl Confirmer for DynConfirmer {
    fn confirm(&self, request: &ConfirmationRequest) -> impl Future<Output = Confirmation> + Send {
        (self.0)(request.clone())
    }
}

fn decorate(
    request: &mut ConfirmationRequest,
    texts: &DialogTexts,
    names: &Names,
    store: &SessionStore,
) {
    if let Some(text) = texts.take(&request.method, &request.params) {
        text.apply(&mut request.summary);
    } else if request.method == "session/agent/update"
        && vw_gate::full_access_escalation(&request.method, &request.params)
    {
        let title = request
            .params
            .get("sessionId")
            .and_then(serde_json::Value::as_str)
            .map_or_else(
                || "Untitled session".to_owned(),
                |id| names.confirmation_title(store, id),
            );
        request.summary = format!(
            "Let the agent in {} run every tool without asking ({})?",
            vw_gate::dialog_chat_quote(&title),
            vw_vocabulary::label("accessModes", "auto-approve")
        );
    } else if request.method == "workspace/git/worktrees/remove" {
        // A removal the core did not prepare (a raw `call`): still a specific sentence.
        request.summary = crate::worktrees_remove::describe_raw(&request.params);
    } else if request.method == "session/delete" {
        let title = request
            .params
            .get("sessionId")
            .and_then(serde_json::Value::as_str)
            .map_or_else(
                || "Untitled session".to_owned(),
                |id| names.confirmation_title(store, id),
            );
        request.summary = format!(
            "Delete session {} and its history permanently (the Vibe CLI loses it too)?",
            vw_gate::dialog_chat_quote(&title),
        );
    } else if let Some(text) = crate::connector_consent::describe(&request.method, &request.params)
    {
        // The gate's generic sentence becomes one that names the server or connector.
        // Who asked is the request's `requester` field (the dialog shows it first), not this text.
        request.summary = text;
    }
}

fn as_confirmation(outcome: &vw_ask::Outcome) -> Confirmation {
    match outcome {
        vw_ask::Outcome::Allowed {
            scope: vw_ask::Scope::Level,
            ..
        } => Confirmation::ApprovedAlways,
        vw_ask::Outcome::Allowed { .. } => Confirmation::Approved,
        _ => Confirmation::Declined,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use serde_json::{Value, json};

    use super::*;

    /// A native dialog that records the methods it is shown and declines every one.
    #[derive(Clone, Default)]
    struct Dialog(Arc<Mutex<Vec<String>>>);

    impl Dialog {
        fn shown(&self) -> Vec<String> {
            self.0.lock().unwrap().clone()
        }
    }

    impl Confirmer for Dialog {
        fn confirm(
            &self,
            request: &ConfirmationRequest,
        ) -> impl Future<Output = Confirmation> + Send {
            self.0.lock().unwrap().push(request.method.clone());
            std::future::ready(Confirmation::Declined)
        }
    }

    fn request(requester: Requester, method: &str, params: Value) -> ConfirmationRequest {
        ConfirmationRequest {
            method: method.to_owned(),
            summary: "Allow this?".to_owned(),
            params,
            requester,
            always: None,
        }
    }

    #[test]
    fn delete_and_full_access_questions_keep_windows_titles_readable() {
        let title = r"C:\Users\Rebeka\Documents\Tests VibeWork";
        let full_access = vw_vocabulary::label("accessModes", "auto-approve");
        let names = Names::default();
        names.note_listing(&[json!({"id": "s1", "title": title})]);
        for (method, params, expected) in [
            (
                "session/delete",
                json!({"sessionId": "s1"}),
                format!(
                    "Delete session \"{title}\" and its history permanently (the Vibe CLI loses it too)?"
                ),
            ),
            (
                "session/agent/update",
                json!({"sessionId": "s1", "agentName": "auto-approve"}),
                format!(
                    "Let the agent in \"{title}\" run every tool without asking ({full_access})?"
                ),
            ),
        ] {
            let mut consent = request(Requester::Window, method, params.clone());
            decorate(
                &mut consent,
                &DialogTexts::default(),
                &names,
                &SessionStore::new(),
            );
            assert_eq!(consent.summary, expected);
            assert_eq!(consent.params, params);
        }
    }

    /// The product rule: through the Debug API only vault actions wait for a person. A vault dialog method goes to
    /// the native dialog (which here declines it); every other confirmation, a Full access escalation included, is
    /// answered by the core with no dialog; the window's own request for the same Full access still goes to the dialog.
    #[tokio::test]
    async fn the_debug_api_is_answered_for_itself_except_for_the_vault_dialogs() {
        let dialog = Dialog::default();
        let confirmer = DynConfirmer::new(
            dialog.clone(),
            Arc::new(Names::default()),
            vw_session::SessionStore::new(),
            crate::ask::Service::new(
                Arc::new(crate::SystemAskClock::default()),
                tokio::sync::broadcast::channel(100).0,
            ),
        );
        for method in crate::vault_scope::DIALOG_METHODS {
            let answer = confirmer
                .confirm(&request(Requester::DebugApi, method, json!({})))
                .await;
            assert_eq!(
                answer,
                Confirmation::Declined,
                "{method} must reach the dialog"
            );
        }
        assert_eq!(
            dialog.shown().len(),
            crate::vault_scope::DIALOG_METHODS.len()
        );

        let full_access = json!({"sessionId": "s1", "agentName": "auto-approve"});
        let default_profile = json!({"sessionId": "s1", "ops": [
            {"op": "set", "path": "/default_agent", "value": "auto-approve"}]});
        let before = dialog.shown().len();
        for (method, params) in [
            ("session/agent/update", full_access.clone()),
            ("config/write", default_profile),
            ("config/proxy/write", json!({})),
            (crate::browser_consent::METHOD, json!({})),
            ("vibe-work/projectInstructions/delete", json!({})),
            ("session/delete", json!({"sessionId": "s1"})),
        ] {
            let answer = confirmer
                .confirm(&request(Requester::DebugApi, method, params))
                .await;
            assert_eq!(answer, Confirmation::Approved, "{method} must not wait");
        }
        assert_eq!(dialog.shown().len(), before, "no dialog for the Debug API");

        let window = confirmer
            .confirm(&request(
                Requester::Window,
                "session/agent/update",
                full_access,
            ))
            .await;
        assert_eq!(window, Confirmation::Declined);
        assert_eq!(
            dialog.shown().last().map(String::as_str),
            Some("session/agent/update")
        );
    }
}
