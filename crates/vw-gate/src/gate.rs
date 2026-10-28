//! The gate: the only door from the UI and the Debug API to Vibe.
//!
//! Depends on: table, class, privileged, `session_rules`, confirm, callbacks, picked, transport,
//! `runtime_view`, error. Called by: the app (UI commands, Debug API routes, the server-request
//! handler for `callback/call`).
//! Invariants:
//! - Default deny: a method that is not in the table never reaches the transport.
//! - Order of checks on [`Gate::call`]: classify, typed params, class rules, confirmation token,
//!   send. The first failing check returns and nothing is written.
//! - The transport is the only outbound path, and the gate holds no lock while awaiting it.
//! - `callback/result` is never sent from [`Gate::call`]; only [`Gate::answer_callback`] sends it,
//!   after the answer matched an open callback's kind and exact shape.
//! - Every refusal is logged with its entry point, method and reason, never with params values
//!   (`log_refusal`).

use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use serde_json::{Value, json};
use tracing::{debug, warn};

use crate::callbacks::{CallbackBook, CallbackKind, OpenCallback};
use crate::class::Class;
use crate::confirm::{
    Clock, Confirmation, ConfirmationRequest, ConfirmationToken, Confirmer, Requester, SystemClock,
    TokenVault,
};
use crate::confirm_text::confirmation_summary;
use crate::error::{CallbackFault, DenyReason, GateError};
use crate::picked::PickedFolders;
use crate::privileged::{Ctx, check_p1};
use crate::runtime_view::model_aliases;
use crate::session_rules::adjust;
use crate::table::row;
use crate::transport::Transport;

/// Methods a trusted surface (the app's own main window) may confirm without the native dialog:
/// commands the user types themselves (`!cmd` -> `session/shellCommand`), the composer access switch,
/// and starting a sign-in the
/// user clicked (`connector_catalog/auth/request` for a Mistral connector, `mcp_catalog/login`
/// for a server already added). A sign-in only produces a page address; the user still opens it in the
/// link dialog and completes it at the vendor, and the token goes to Mistral or Vibe's keyring, never
/// through the app. Everything else that needs a confirmation (installs, new servers such as
/// `mcp_catalog/add`, data leaving the machine, other access-mode changes) always goes through the native
/// dialog. Which origins may use this list is the core's decision (`own_token`).
pub const TRUSTED_SURFACE_METHODS: &[&str] = &[
    "session/shellCommand",
    "session/agent/update",
    "connector_catalog/auth/request",
    "mcp_catalog/login",
];

/// Installs the window may confirm with an install offer it showed first: skills Install and Update.
/// The core mints the token through [`Gate::request_offered_confirmation`] only after taking the single-use offer
/// the user saw (name, source, licence, what runs); without an offer these keep the native dialog. Turning a
/// plugin on and every delete are not here.
pub const OFFERED_INSTALL_METHODS: &[&str] = &["skills/import", "skills/setVersion"];

/// Tuning of a gate. `Default` gives production values.
#[derive(Clone)]
#[non_exhaustive]
pub struct GateConfig {
    /// How long a confirmation token stays valid. Default 60 s.
    pub token_ttl: Duration,
    /// How many tokens (live and spent) are remembered; the oldest is dropped first. Default 64.
    pub max_tokens: usize,
    /// How many callbacks may be open at once. Default 1024.
    pub max_open_callbacks: usize,
    /// Time source; tests replace it.
    pub clock: Arc<dyn Clock>,
}

impl GateConfig {
    /// Production values with a custom clock.
    #[must_use]
    pub fn with_clock(clock: Arc<dyn Clock>) -> Self {
        Self {
            token_ttl: Duration::from_secs(60),
            max_tokens: 64,
            max_open_callbacks: 1024,
            clock,
        }
    }
}

impl Default for GateConfig {
    fn default() -> Self {
        Self::with_clock(Arc::new(SystemClock))
    }
}

/// What a successful call returns.
#[derive(Debug, Clone, PartialEq)]
pub struct Reply {
    /// The server's result.
    pub result: Value,
    /// For `turn/start` and `turn/steer`: the `clientUserMessageId` that was sent (the caller's or
    /// ours).
    pub client_user_message_id: Option<String>,
    /// For `session/turn/enqueue`: the `entryId` sent on each user entry, in order (empty for
    /// every other method).
    pub user_entry_ids: Vec<String>,
}

/// The validated call, ready to send.
struct Plan {
    /// The params to send.
    params: Value,
    client_user_message_id: Option<String>,
    user_entry_ids: Vec<String>,
    /// Set when the call needs a native confirmation first.
    confirm: Option<String>,
}

struct State {
    vault: TokenVault,
    callbacks: CallbackBook,
    aliases: HashSet<String>,
    picked: PickedFolders,
}

/// The method gate. Cheap to share behind an `Arc`; all methods take `&self`.
pub struct Gate<T, C> {
    transport: T,
    confirmer: C,
    clock: Arc<dyn Clock>,
    state: Mutex<State>,
}

impl<T: Transport, C: Confirmer> Gate<T, C> {
    /// Builds a gate over `transport`; `confirmer` shows the native dialogs for P2 calls.
    #[must_use]
    pub fn new(transport: T, confirmer: C, config: GateConfig) -> Self {
        Self {
            transport,
            confirmer,
            clock: config.clock,
            state: Mutex::new(State {
                vault: TokenVault::new(config.token_ttl, config.max_tokens),
                callbacks: CallbackBook::new(config.max_open_callbacks),
                aliases: HashSet::new(),
                picked: PickedFolders::default(),
            }),
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        // A panic while holding the lock cannot leave the state half-updated in a way that
        // matters here (every update is a single insert or remove), so poisoning is ignored.
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Sends `method` to Vibe if the gate allows it.
    ///
    /// `token` is the confirmation token for calls that need one; it is ignored for calls that do
    /// not.
    ///
    /// # Errors
    /// [`GateError::Denied`] for denied and unknown methods and policy violations;
    /// [`GateError::InvalidParams`] for params outside the typed shape;
    /// [`GateError::ConfirmationRequired`] / [`GateError::ConfirmationRejected`] when a needed
    /// token is missing or not accepted; [`GateError::Transport`] when the send fails. Every error
    /// except `Transport` means nothing was sent.
    pub async fn call(
        &self,
        method: &str,
        params: Value,
        token: Option<&ConfirmationToken>,
    ) -> Result<Reply, GateError> {
        let result = self.call_unlogged(method, params, token).await;
        if let Err(error) = &result {
            log_refusal("call", method, error);
        }
        result
    }

    /// Like [`Gate::call`], but when the server refuses the forwarded request with an error `retry` accepts, the
    /// SAME request is sent once more after `pause`: the user's confirmation covered that request, so it is spent
    /// once, never redeemed again (a second [`Gate::call`] would be refused as "token already used"). For a
    /// refusal the server reports as passing (on Windows a file of the session read by someone else for a moment).
    ///
    /// # Errors
    /// As [`Gate::call`]; the second answer is returned when there was one.
    pub async fn call_with_retry<F, P, Fut>(
        &self,
        method: &str,
        params: Value,
        token: Option<&ConfirmationToken>,
        retry: F,
        pause: P,
    ) -> Result<Reply, GateError>
    where
        F: Fn(&GateError) -> bool,
        P: FnOnce() -> Fut,
        Fut: std::future::Future<Output = ()>,
    {
        let result = async {
            let plan = self.authorise(method, &params, token)?;
            let first = self.transport.request(method, plan.params.clone()).await;
            let result = match first.map_err(GateError::from) {
                Err(error) if retry(&error) => {
                    warn!(method, %error, "gate: the server refused for a moment; the same request once more");
                    pause().await;
                    self.transport.request(method, plan.params.clone()).await?
                }
                other => other?,
            };
            Ok(self.answered(method, plan, result))
        }
        .await;
        if let Err(error) = &result {
            log_refusal("call", method, error);
        }
        result
    }

    async fn call_unlogged(
        &self,
        method: &str,
        params: Value,
        token: Option<&ConfirmationToken>,
    ) -> Result<Reply, GateError> {
        let plan = self.authorise(method, &params, token)?;
        debug!(method, "gate: forwarding");
        let result = self.transport.request(method, plan.params.clone()).await?;
        Ok(self.answered(method, plan, result))
    }

    /// The plan for a call the gate will forward: shape and policy checked, and its confirmation token redeemed
    /// (spent) when the call needs one.
    fn authorise(
        &self,
        method: &str,
        params: &Value,
        token: Option<&ConfirmationToken>,
    ) -> Result<Plan, GateError> {
        let plan = self.plan(method, params)?;
        if let Some(summary) = plan.confirm.clone() {
            let Some(token) = token else {
                return Err(GateError::ConfirmationRequired {
                    method: method.to_owned(),
                    summary,
                });
            };
            let now = self.clock.now();
            // Bound to the caller's params, not the adjusted ones: that is what the user saw.
            self.state()
                .vault
                .redeem(method, params, token, now)
                .map_err(|fault| GateError::ConfirmationRejected {
                    method: method.to_owned(),
                    fault,
                })?;
        }
        Ok(plan)
    }

    /// The reply for a forwarded call's result (and what the gate learns from it).
    fn answered(&self, method: &str, plan: Plan, result: Value) -> Reply {
        self.learn_from_result(method, &result);
        Reply {
            result,
            client_user_message_id: plan.client_user_message_id,
            user_entry_ids: plan.user_entry_ids,
        }
    }

    /// Sends a request the gate itself built from a typed handler (`tool_rules`), skipping the client table.
    /// Crate-internal: no client command reaches it.
    pub(crate) async fn send_unchecked(
        &self,
        method: &str,
        params: Value,
    ) -> Result<Reply, GateError> {
        debug!(method, "gate: forwarding a gate-built request");
        let result = self.transport.request(method, params).await?;
        Ok(Reply {
            result,
            client_user_message_id: None,
            user_entry_ids: Vec::new(),
        })
    }

    /// Answers whether [`Gate::call`] would forward `method` with `params` and `token`, without
    /// sending anything, spending the token or changing any state. The core uses it before a step
    /// with a side effect that the call needs first (opening a session's channel), so a
    /// call the gate refuses causes nothing at all.
    ///
    /// # Errors
    /// The refusal [`Gate::call`] would return (never `Transport`). A token that expires between
    /// this check and the call is still refused by the call.
    pub fn check(
        &self,
        method: &str,
        params: &Value,
        token: Option<&ConfirmationToken>,
    ) -> Result<(), GateError> {
        let plan = self.plan(method, params)?;
        let Some(summary) = plan.confirm else {
            return Ok(());
        };
        let Some(token) = token else {
            return Err(GateError::ConfirmationRequired {
                method: method.to_owned(),
                summary,
            });
        };
        let now = self.clock.now();
        self.state()
            .vault
            .check(method, params, token, now)
            .map_err(|fault| GateError::ConfirmationRejected {
                method: method.to_owned(),
                fault,
            })
    }

    /// Asks the user, in the native dialog, to approve one exact call, and returns the token that
    /// lets [`Gate::call`] send it once.
    ///
    /// The params are validated first, so the user is never asked about a call that could not
    /// pass.
    ///
    /// # Errors
    /// The same refusals as [`Gate::call`] for the method and params;
    /// [`GateError::ConfirmationNotApplicable`] when the call needs no confirmation;
    /// [`GateError::ConfirmationDeclined`] when the user said no.
    pub async fn request_confirmation(
        &self,
        method: &str,
        params: &Value,
    ) -> Result<ConfirmationToken, GateError> {
        // Who asked is unknown here: the window's dialog is shown (`DebugApi` now means the request
        // is answered without a dialog by the core's confirmer, so an unknown caller must never get it).
        self.request_confirmation_as(method, params, Requester::Window)
            .await
    }

    /// [`Gate::request_confirmation`] for a known requester, which the dialog shows first.
    /// Who asks is a structured field, never read from the summary's wording.
    ///
    /// # Errors
    /// As [`Gate::request_confirmation`].
    pub async fn request_confirmation_as(
        &self,
        method: &str,
        params: &Value,
        requester: Requester,
    ) -> Result<ConfirmationToken, GateError> {
        let result = self
            .request_confirmation_unlogged(method, params, requester)
            .await;
        if let Err(error) = &result {
            log_refusal("request_confirmation", method, error);
        }
        result
    }

    async fn request_confirmation_unlogged(
        &self,
        method: &str,
        params: &Value,
        requester: Requester,
    ) -> Result<ConfirmationToken, GateError> {
        let plan = self.plan(method, params)?;
        let Some(summary) = plan.confirm else {
            return Err(GateError::ConfirmationNotApplicable {
                method: method.to_owned(),
            });
        };
        let request = ConfirmationRequest {
            method: method.to_owned(),
            summary: confirmation_summary(method, params, &summary),
            params: params.clone(),
            requester,
            always: None,
        };
        match self.confirmer.confirm(&request).await {
            Confirmation::Approved | Confirmation::ApprovedAlways => {
                let now = self.clock.now();
                self.state().vault.mint(method, params, now)
            }
            Confirmation::Declined => Err(GateError::ConfirmationDeclined {
                method: method.to_owned(),
            }),
        }
    }

    /// Mints the confirmation token for one exact call WITHOUT showing the native dialog, for a
    /// caller that vouches for the user's intent because the request came from a trusted surface
    /// (the main window renders no untrusted active content, so a command the user typed
    /// there needs no extra dialog, like in a terminal agent).
    ///
    /// Only the methods in [`TRUSTED_SURFACE_METHODS`] may be confirmed this way; everything that
    /// installs code, adds endpoints or sends data off the machine keeps its native dialog. The
    /// token is bound to the exact method and params and single use, like any other.
    ///
    /// The caller (the core) decides who counts as trusted; the gate only refuses methods that
    /// no surface may self-confirm.
    ///
    /// # Errors
    /// [`GateError::Denied`] when `method` is not in [`TRUSTED_SURFACE_METHODS`]; the same
    /// refusals as [`Gate::call`] for the params; [`GateError::ConfirmationNotApplicable`] when
    /// the call needs no confirmation (an interrupt).
    pub fn request_trusted_confirmation(
        &self,
        method: &str,
        params: &Value,
    ) -> Result<ConfirmationToken, GateError> {
        let result = self.trusted_confirmation_unlogged(method, params);
        if let Err(error) = &result {
            log_refusal("request_trusted_confirmation", method, error);
        }
        result
    }

    /// Mints the confirmation token for one exact install WITHOUT the native dialog, for a window click that
    /// confirmed an install offer the window showed first. Only [`OFFERED_INSTALL_METHODS`] qualify; the
    /// core vouches that the offer was shown and taken (`vw-core-api` `install_offers`). Bound to the exact
    /// method and params and single use, like any other token.
    ///
    /// # Errors
    /// [`GateError::Denied`] for any other method; the same refusals as [`Gate::call`] for the params.
    pub fn request_offered_confirmation(
        &self,
        method: &str,
        params: &Value,
    ) -> Result<ConfirmationToken, GateError> {
        let result = if OFFERED_INSTALL_METHODS.contains(&method) {
            self.plan(method, params).and_then(|plan| {
                if plan.confirm.is_none() {
                    return Err(GateError::ConfirmationNotApplicable {
                        method: method.to_owned(),
                    });
                }
                let now = self.clock.now();
                self.state().vault.mint(method, params, now)
            })
        } else {
            Err(denied(
                method,
                DenyReason::Policy("only a shown install offer may skip the dialog".to_owned()),
            ))
        };
        if let Err(error) = &result {
            log_refusal("request_offered_confirmation", method, error);
        }
        result
    }

    fn trusted_confirmation_unlogged(
        &self,
        method: &str,
        params: &Value,
    ) -> Result<ConfirmationToken, GateError> {
        if !TRUSTED_SURFACE_METHODS.contains(&method) {
            return Err(denied(
                method,
                DenyReason::Policy(
                    "no surface may confirm this method without the dialog".to_owned(),
                ),
            ));
        }
        let plan = self.plan(method, params)?;
        if plan.confirm.is_none() {
            return Err(GateError::ConfirmationNotApplicable {
                method: method.to_owned(),
            });
        }
        let now = self.clock.now();
        self.state().vault.mint(method, params, now)
    }

    /// Classifies and validates a call without sending anything.
    fn plan(&self, method: &str, params: &Value) -> Result<Plan, GateError> {
        let Some(row) = row(method) else {
            return Err(denied(method, DenyReason::Unclassified));
        };
        let typed = |check: Option<crate::table::Check>| -> Result<(), GateError> {
            match check {
                Some(check) => check(params).map_err(|detail| GateError::InvalidParams {
                    method: method.to_owned(),
                    detail,
                }),
                None => Ok(()),
            }
        };
        match &row.class {
            Class::Denied(reason) => Err(denied(method, reason.clone())),
            Class::Forwarded => {
                typed(row.check)?;
                let adjusted = {
                    let state = self.state();
                    adjust(method, params, &state.picked)?
                };
                Ok(Plan {
                    params: adjusted.params,
                    client_user_message_id: adjusted.client_user_message_id,
                    user_entry_ids: adjusted.user_entry_ids,
                    confirm: adjusted.confirm,
                })
            }
            Class::Confirmed => {
                typed(row.check)?;
                Ok(Plan {
                    params: params.clone(),
                    client_user_message_id: None,
                    user_entry_ids: Vec::new(),
                    confirm: Some(
                        "This action changes code, access, or a remote connection.".to_owned(),
                    ),
                })
            }
            Class::Privileged(kind) => {
                let confirm = {
                    let state = self.state();
                    let ctx = Ctx {
                        aliases: &state.aliases,
                        picked: &state.picked,
                    };
                    check_p1(*kind, method, params, &ctx)?
                };
                Ok(Plan {
                    params: params.clone(),
                    client_user_message_id: None,
                    user_entry_ids: Vec::new(),
                    confirm,
                })
            }
        }
    }

    /// Keeps the known model aliases current from results that carry a runtime snapshot.
    fn learn_from_result(&self, method: &str, result: &Value) {
        if matches!(
            method,
            "runtime/read" | "config/write" | "config/model/write"
        ) {
            self.observe_runtime(result);
        }
    }

    /// Replaces the known model aliases with those in a runtime snapshot (the result of
    /// `runtime/read`, or the `runtime/updated` notification params). Values without a model list
    /// leave the known set unchanged.
    pub fn observe_runtime(&self, snapshot: &Value) {
        if let Some(aliases) = model_aliases(snapshot) {
            self.state().aliases = aliases;
        }
    }

    /// Sets the configured model aliases directly (the core may know them from its own state).
    pub fn set_model_aliases(&self, aliases: impl IntoIterator<Item = String>) {
        self.state().aliases = aliases.into_iter().collect();
    }

    /// Records a folder the user picked in a native dialog. Only such folders (and folders inside
    /// them) can be trusted through the gate.
    ///
    /// # Errors
    /// [`GateError::InvalidFolder`] when the path does not exist or is not a directory.
    pub fn register_picked_folder(&self, path: &Path) -> Result<(), GateError> {
        self.state().picked.register(path)
    }

    /// True when `path` is a folder the user picked in a native dialog in this run, or lies inside one
    /// (both resolved, so `..` hops and symlinks out of a picked folder do not count). A relative or
    /// unresolvable path is never picked. The `worktrees` command acts only on such folders
    /// unless the user confirms in the native dialog.
    #[must_use]
    pub fn is_picked_folder(&self, path: &Path) -> bool {
        self.state().picked.contains_path(path)
    }

    /// The folders the user picked in a native dialog in this run (canonical paths). The file
    /// resolver compares a candidate with these LEXICALLY before any filesystem access (no oracle for paths
    /// outside them), then checks the canonical path again.
    #[must_use]
    pub fn picked_roots(&self) -> Vec<std::path::PathBuf> {
        self.state().picked.roots()
    }

    /// The picked folders whose spelling as picked (the folder dialog's, the Debug API's) differs from their canonical
    /// path, as `(spelling, canonical)`: macOS `/tmp/x` for `/private/tmp/x`, a folder reached through a symbolic link.
    /// For the same lexical comparison as [`Self::picked_roots`] in the spelling the window keeps; never for trust.
    #[must_use]
    pub fn picked_spellings(&self) -> Vec<(std::path::PathBuf, std::path::PathBuf)> {
        self.state().picked.spellings()
    }

    /// Registers the callback of an inbound `callback/call` (pass its `params`). The UI should be
    /// shown every registered callback, including kinds the gate will not answer.
    ///
    /// # Errors
    /// [`CallbackFault::InvalidRegistration`] when `params` is not a callback entry;
    /// [`CallbackFault::TooManyOpen`] past the configured limit;
    /// [`CallbackFault::DuplicateId`] for an id that was already answered.
    pub fn register_callback(&self, params: &Value) -> Result<CallbackKind, CallbackFault> {
        self.state().callbacks.register(params)
    }

    /// Checks an original answer before the core replaces an approval with a refusal. Does not claim it.
    ///
    /// # Errors
    /// [`GateError::Callback`] when the session, callback kind or offered output shape does not match.
    pub fn validate_callback_answer(
        &self,
        session_id: &str,
        callback_id: &str,
        output: &Value,
    ) -> Result<(), GateError> {
        self.state()
            .callbacks
            .validate(session_id, callback_id, output)
            .map_err(|fault| {
                let error = GateError::Callback(fault);
                log_refusal("answer_callback", "callback/result", &error);
                error
            })
    }

    /// Answers an open callback with `output` (the `output` object of `callback/result`).
    ///
    /// The answer is sent only if it matches the callback's session and kind and has exactly the
    /// output shape for that kind. The callback is claimed before the send is awaited,
    /// so concurrent answers send once. A claimed callback is never answered again, even if the
    /// send fails.
    ///
    /// # Errors
    /// [`GateError::Callback`] with the reason when the answer is refused (nothing was sent);
    /// [`GateError::Transport`] when the send fails.
    pub async fn answer_callback(
        &self,
        session_id: &str,
        callback_id: &str,
        output: Value,
    ) -> Result<Value, GateError> {
        if let Err(fault) = self
            .state()
            .callbacks
            .claim(session_id, callback_id, &output)
        {
            let error = GateError::Callback(fault);
            log_refusal("answer_callback", "callback/result", &error);
            return Err(error);
        }
        let params = json!({
            "sessionId": session_id,
            "result": {"callbackId": callback_id, "output": output},
        });
        let sent = self.transport.request("callback/result", params).await;
        self.state().callbacks.sent(session_id);
        match sent {
            Ok(result) => Ok(result),
            Err(error) => {
                warn!(
                    callback_id,
                    "gate: callback answer was claimed but the send failed"
                );
                Err(error.into())
            }
        }
    }

    /// Forgets a callback the server closed (cancelled, expired, or its turn ended).
    pub fn callback_closed(&self, callback_id: &str) {
        self.state().callbacks.close(callback_id);
    }

    /// Forgets every callback of a session (the session ended or the server restarted).
    pub fn session_closed(&self, session_id: &str) {
        self.state().callbacks.close_session(session_id);
    }

    /// The callbacks waiting for an answer, for the UI to show.
    #[must_use]
    pub fn open_callbacks(&self) -> Vec<OpenCallback> {
        self.state().callbacks.open()
    }

    /// True while an answer of this session is still being delivered.
    pub fn answering(&self, session_id: &str) -> bool {
        self.state().callbacks.sending(session_id)
    }
}

/// Longest method name or refusal text written to the log; a hostile caller can send megabytes.
const LOG_TEXT_MAX: usize = 200;

/// Logs a refusal: the entry point, the method and why, never the params (they can hold prompts,
/// paths or keys). Caller-chosen text (the method name, an unknown key or callback id inside the
/// reason) is truncated and written with `Debug`, which escapes control characters, so a method
/// name cannot forge log lines. A missing confirmation is the normal first step of a P2 call and
/// logs at debug; transport failures are not refusals and are left to the caller.
fn log_refusal(entry: &'static str, method: &str, error: &GateError) {
    let short = |text: &str| text.chars().take(LOG_TEXT_MAX).collect::<String>();
    let method = short(method);
    let reason = match error {
        GateError::Transport(_) => return,
        GateError::ConfirmationRequired { .. } => {
            debug!(entry, method = ?method, "gate: needs a native confirmation");
            return;
        }
        GateError::Denied { reason, .. } => format!("denied: {reason}"),
        GateError::InvalidParams { detail, .. } => format!("invalid params: {detail}"),
        GateError::ConfirmationRejected { fault, .. } => format!("token refused: {fault}"),
        GateError::ConfirmationDeclined { .. } => "declined by the user".to_owned(),
        GateError::ConfirmationNotApplicable { .. } => "needs no confirmation".to_owned(),
        GateError::Callback(fault) => format!("answer refused: {fault}"),
        GateError::InvalidFolder { .. } => "invalid folder".to_owned(),
        GateError::Entropy(_) => "no randomness available".to_owned(),
    };
    warn!(entry, method = ?method, reason = ?short(&reason), "gate: refused");
}

fn denied(method: &str, reason: DenyReason) -> GateError {
    GateError::Denied {
        method: method.to_owned(),
        reason,
    }
}
