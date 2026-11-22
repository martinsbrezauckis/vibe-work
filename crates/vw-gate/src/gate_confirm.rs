//! Native and trusted confirmations on the gate: proxy-forwarding consent, confirmation requests through
//! the confirmer, trusted-surface and offered-install confirmations, and restored Full access.

use serde_json::{Value, json};

use crate::confirm::{Confirmation, ConfirmationRequest, ConfirmationToken, Confirmer, Requester};
use crate::confirm_text::confirmation_summary;
use crate::error::{DenyReason, GateError};
use crate::transport::Transport;

use super::{Gate, OFFERED_INSTALL_METHODS, TRUSTED_SURFACE_METHODS, denied, log_refusal};

impl<T: Transport, C: Confirmer> Gate<T, C> {
    /// Spend the original proxy-write consent and bind it to the core's credential-free forwarding call.
    /// The core stores the sign-in in its vault; only the loopback address reaches Vibe. No second dialog is shown.
    /// # Errors
    /// Either shape is invalid, or the original single-use confirmation is absent, expired or mismatched.
    pub fn confirm_proxy_forwarding(
        &self,
        original: &Value,
        forwarded: &Value,
        token: Option<&ConfirmationToken>,
    ) -> Result<ConfirmationToken, GateError> {
        let method = "config/proxy/write";
        self.plan(method, forwarded)?;
        self.authorise(method, original, token)?;
        self.state().vault.mint(method, forwarded, self.clock.now())
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

    /// Reuses the person's earlier Full access choice for this exact session. The core must have
    /// that choice in its own access record, never from caller params or Vibe's writable history.
    /// This fixed resume shape cannot add config, tools, endpoints or a different profile; ordinary
    /// session/resume requests still require the native dialog. The returned token is single use.
    ///
    /// # Errors
    /// The same validation and token-allocation refusals as [`Gate::request_confirmation`].
    pub fn confirm_restored_full_access(
        &self,
        session_id: &str,
    ) -> Result<(Value, ConfirmationToken), GateError> {
        let method = "session/resume";
        let params =
            json!({"sessionId":session_id,"agentConfig":{"approvalPolicy":"auto-approve"}});
        self.plan(method, &params)?;
        let token = self.state().vault.mint(method, &params, self.clock.now())?;
        Ok((params, token))
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
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use vw_rpc::RpcError;

    use crate::{Confirmation, ConfirmationRequest, Confirmer, Gate, GateConfig, Transport};

    struct NeverTransport;

    impl Transport for NeverTransport {
        fn request(
            &self,
            _method: &str,
            _params: Value,
        ) -> impl std::future::Future<Output = Result<Value, RpcError>> + Send {
            std::future::poll_fn(|_| panic!("restored access test never sends a request"))
        }
    }

    struct NeverConfirm;

    impl Confirmer for NeverConfirm {
        fn confirm(
            &self,
            _request: &ConfirmationRequest,
        ) -> impl std::future::Future<Output = Confirmation> + Send {
            std::future::poll_fn(|_| panic!("restored access test never opens a dialog"))
        }
    }

    #[test]
    fn restored_full_access_uses_vibe314_policy_field_and_exact_token() {
        let gate = Gate::new(NeverTransport, NeverConfirm, GateConfig::default());
        let (params, token) = gate
            .confirm_restored_full_access("saved-chat")
            .expect("app-owned authority record makes a gated resume");
        assert_eq!(
            params,
            json!({"sessionId":"saved-chat","agentConfig":{"approvalPolicy":"auto-approve"}})
        );
        assert!(gate.check("session/resume", &params, Some(&token)).is_ok());
        for changed in [
            json!({"sessionId":"another-chat","agentConfig":{"approvalPolicy":"auto-approve"}}),
            json!({"sessionId":"saved-chat","agentConfig":{"agent":"default","approvalPolicy":"auto-approve"}}),
        ] {
            assert!(
                gate.check("session/resume", &changed, Some(&token))
                    .is_err()
            );
        }
    }
}
