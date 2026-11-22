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
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use vw_platform::sync::LockRecover;

use serde_json::Value;
use tracing::{debug, warn};

use crate::callbacks::CallbackBook;
use crate::class::Class;
use crate::confirm::{Clock, ConfirmationToken, Confirmer, SystemClock, TokenVault};
use crate::error::{DenyReason, GateError};
use crate::picked::PickedFolders;
use crate::privileged::{Ctx, check_p1};
use crate::runtime_view::model_aliases;
use crate::session_rules::adjust;
use crate::table::row;
use crate::transport::Transport;

#[path = "gate_confirm.rs"]
mod confirmations;
#[path = "gate_callbacks.rs"]
mod open_callbacks;

/// Methods a trusted surface (the app's own main window) may confirm without the native dialog:
/// commands the user types themselves (`!cmd` -> `session/shellCommand`), the composer access switch,
/// starting a sign-in the
/// user clicked (`connector_catalog/auth/request` for a Mistral connector, `mcp_catalog/login`
/// for a server already added), and Connect of a Mistral connector that needs no sign-in
/// (`connector_catalog/connect`). A sign-in only produces a page address; the user still opens it in the
/// link dialog and completes it at the vendor, and the token goes to Mistral or Vibe's keyring, never
/// through the app. Connect stores no secret (the key proxy refuses any credential value). Everything else that needs a confirmation (installs, new servers such as
/// `mcp_catalog/add`, data leaving the machine, other access-mode changes) always goes through the native
/// dialog. Which origins may use this list is the core's decision (`own_token`).
pub const TRUSTED_SURFACE_METHODS: &[&str] = &[
    "session/shellCommand",
    "session/approvalPolicy/update",
    "connector_catalog/auth/request",
    "mcp_catalog/login",
    "connector_catalog/connect",
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
        self.state.lock_or_recover()
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
