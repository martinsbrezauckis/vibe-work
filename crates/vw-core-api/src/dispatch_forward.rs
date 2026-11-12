//! Forwarded commands, confirmation tokens and profile restoration for the core dispatcher.
//! Depends on the existing gate, channels and session projection; called by Core dispatch methods.
//! The order of authority checks, transport calls and result projection is unchanged.
use crate::command::{CommandResult, Origin};
use crate::core::Core;
use crate::error::{CoreError, ErrorCode};
use crate::sessions;
use serde_json::Value;
use vw_gate::{ConfirmationToken, GateError, Reply, TRUSTED_SURFACE_METHODS};

impl Core {
    /// A call whose result is a session state: forwarded, loaded, answered with the snapshot.
    pub(crate) async fn session_call(
        &self,
        origin: Origin,
        method: &str,
        params: Value,
    ) -> Result<CommandResult, CoreError> {
        // A chat start never waits on gh (it can hang on a keyring prompt or without network).
        if method == "session/start" && self.shared.github_reads.enabled() {
            self.shared.github_reads.refresh_later();
        }
        let reply = self.forward(origin, method, params, None).await?;
        let Some(id) = reply
            .result
            .pointer("/state/session/id")
            .and_then(Value::as_str)
        else {
            return Err(CoreError::bad_result(method, "state.session.id"));
        };
        Ok(CommandResult::Session {
            snapshot: sessions::snapshot(&self.shared, id)?,
            detail: sessions::detail(&reply.result),
        })
    }

    /// A client's resume names the resolved profile and is followed by the profile restore (`restore.rs`, fail
    /// closed): to the profile it named, else the session's. With a confirmation token (`confirmed`) the params stay
    /// exactly as the user approved them; an unpinned confirmed resume is refused. `None` for other methods.
    async fn resume_restore(
        &self,
        method: &str,
        params: &mut Value,
        confirmed: bool,
    ) -> Result<Option<crate::restore::Restoring>, CoreError> {
        let Some(id) = params
            .get("sessionId")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .filter(|_| method == "session/resume")
        else {
            return Ok(None);
        };
        let asked = params
            .pointer("/agentConfig/agent")
            .and_then(Value::as_str)
            .map(str::to_owned);
        if confirmed && asked.is_none() {
            return Err(CoreError::new(
                ErrorCode::InvalidParams,
                "a confirmed resume must name its agent; request confirmation with the session's access mode",
            )
            .with_method(method));
        }
        let restore = self.begin_restore(&id, asked).await;
        if !confirmed {
            crate::access::set_resume_profile(params, restore.profile());
        }
        Ok(Some(restore))
    }

    /// The gate call plus its store effect and the profile-switch check.
    pub(crate) async fn forward(
        &self,
        origin: Origin,
        method: &str,
        params: Value,
        token: Option<String>,
    ) -> Result<Reply, CoreError> {
        self.forward_with_access_switch(origin, method, params, token, false, None)
            .await
    }

    /// [`Self::forward`] for an app continuation: sent only while the chat's Stop generation is still
    /// `generation`, checked once the chat's send lock is held (`stops.rs`); otherwise nothing is sent.
    pub(crate) async fn forward_unless_stopped(
        &self,
        method: &str,
        params: Value,
        generation: u64,
    ) -> Result<Reply, CoreError> {
        self.forward_with_access_switch(Origin::Ui, method, params, None, false, Some(generation))
            .await
    }

    /// The typed composer switch is the sole route that can use the trusted-window Full access
    /// exception. A raw `call` of the same method must still bring a native confirmation token.
    pub(super) async fn forward_with_access_switch(
        &self,
        origin: Origin,
        method: &str,
        params: Value,
        token: Option<String>,
        trusted_access_switch: bool,
        stop: Option<u64>,
    ) -> Result<Reply, CoreError> {
        let mut params = params;
        let selected = crate::model_choices::selected(&self.shared, method, &mut params);
        if let Some(reply) = crate::stop_tree::interrupt(&self.shared, method, &params).await? {
            return Ok(reply);
        }
        crate::session_ownership::check_delete(&self.shared, method, &params).await?;
        let _refresh = self.plugin_refresh_lock(method, &params, stop).await?;
        // Under the chat's send lock (`stops.rs`): a Stop's interrupt reaches a turn a checked send started; a Stop
        // since the continuation was scheduled refuses it here, before a stopped chat is reopened (the test that
        // decides runs again right before the send, `turn_start::call`).
        let params = crate::stop_tree::admit(&self.shared, method, params, stop).await?;
        // A stuck session is never opened implicitly (`stuck.rs`): refused at once, nothing sent.
        let opens = crate::routing::attaches_session(method, &params).or_else(|| {
            (method == "session/resume")
                .then(|| params["sessionId"].as_str())
                .flatten()
        });
        if let Some(id) = opens.filter(|id| self.shared.stuck.is_stuck(id)) {
            return Err(crate::stuck::refusal(id, method));
        }
        // The `vw-plan` profile names this session's folder before Vibe opens the session.
        crate::plan_profile::before_open(&self.shared, method, &params);
        let mut token = token.map(ConfirmationToken::from_wire);
        // A client's resume names the resolved profile and is followed by the profile restore
        // (`restore.rs`, fail closed): to the profile it named, else the session's. The session counts
        // as "being restored" until then. With a confirmation token the params stay exactly as the
        // user approved them; an unpinned confirmed resume is refused.
        let mut params = params;
        let restore = self
            .resume_restore(method, &mut params, token.is_some())
            .await?;
        if token.is_none() && method != "config/model/write" {
            token = self
                .own_token(origin, method, &params, trusted_access_switch)
                .await?;
        }
        // A stale picker is resolved against the current session in `pick`, after an implicit reopen.
        // The preliminary check still validates the session-only write before attaching a channel.
        let admission_params = crate::model_choices::admission(method, &params);
        self.ensure_command_channel(origin, method, &admission_params, token.as_ref())
            .await?;
        self.shared.channels.canonical_params(method, &mut params);
        // Attach first: reopening takes this same restore lock. Then read only the verified runtime.
        let _runtime_restore = self.runtime_restore_lock(method, &params).await;
        Box::pin(crate::model_choices::before(&self.shared, method, &params)).await;
        let _skills_guard = self.lock_skills_work(method).await;
        // The session's review baseline exists before Vibe starts working in its folder.
        if crate::changes::starts_work(method) {
            self.changes_before_work(&params).await;
        }
        // Refusals precede filesystem inspection. Capture authority before Vibe opens the session.
        if matches!(
            method,
            "session/start" | "session/resume" | "session/continue" | "session/fork"
        ) {
            self.shared
                .gate
                .check(method, &params, token.as_ref())
                .map_err(|e| CoreError::from_gate(method, &e))?;
        }
        let entry_project = crate::entry_projects::before_open(&self.shared, method, &params).await;
        // Persist only byte-carrying input the gate has accepted; filenames never grant read access.
        if method != "config/model/write" {
            self.prepare_model_input(method, &mut params, token.as_ref())
                .await?;
        }
        let (compaction, _in) = crate::dispatch_compaction::channel(&self.shared, method, &params);
        // Full access before a switch or a default write (after any reopen above): the alert is for a raise.
        let was_full = self.full_before(method, &params);
        let deleted_children =
            crate::session_end::deletion_children(&self.shared, method, &params).await;
        let mut reply = Box::pin(self.send_call(crate::dispatch_call::Sending {
            method,
            params: &params,
            token: token.as_ref(),
            stop,
            opening: restore.is_some(),
            compaction,
            selected,
        }))
        .await?;
        // Before the store, the name cache or any client sees it: chat titles hold no vault value (`title_guard`).
        crate::title_guard::withhold_in(&self.shared, &mut reply.result);
        if let Some(opening) = entry_project {
            opening.finish(&self.shared, &reply.result);
        }
        if method == "session/fork" {
            crate::model_choices::forked(&self.shared, &params, &reply.result);
        }
        if matches!(method, "runtime/read" | "config/reload")
            && let Some(id) = params["sessionId"].as_str()
            && self.shared.channels.has_channel(id)
            && let Some(runtime) = reply.result.get("runtime")
        {
            crate::model_choices::publish(&self.shared, id, runtime.clone()).await;
        }
        if method == "turn/interrupt"
            && let Some(id) = params["sessionId"].as_str()
        {
            crate::stop_tree::descendants(&self.shared, id).await?;
        }
        crate::session_loops::after(&self.shared, method, &params).await;
        sessions::project_result(&self.shared, method, &params, &mut reply.result);
        // A delete ends the session here, in one place (`session_end.rs`).
        sessions::observe(&self.shared, method, &params, &reply.result).await?;
        for child in deleted_children {
            crate::session_end::deleted(&self.shared, &child).await;
        }
        self.plugins_after_open(origin, restore, method, &params, &reply.result)
            .await?;
        if let Some(was_full) = was_full {
            self.after_access_write(origin, method, &params, token.is_none(), was_full)
                .await;
        }
        sessions::project_client(&self.shared, method, &params, &mut reply.result);
        Ok(reply)
    }

    /// Refuse before opening or leasing a channel, then attach only when this command needs it.
    async fn ensure_command_channel(
        &self,
        origin: Origin,
        method: &str,
        params: &Value,
        token: Option<&ConfirmationToken>,
    ) -> Result<(), CoreError> {
        if let Some(id) = crate::routing::attaches_session(method, params)
            && !self.shared.channels.has_channel(id)
        {
            self.shared
                .gate
                .check(method, params, token)
                .map_err(|e| CoreError::from_gate(method, &e))?;
            self.bounded_open(id, self.attach_session(origin, id))
                .await?;
        }
        Ok(())
    }

    /// The confirmation token the core supplies itself for a call that came without one, or `None`:
    /// - the window (trusted surface): for `TRUSTED_SURFACE_METHODS` only, without a dialog;
    /// - the Debug API (it acts as the user): for calls that need a confirmation. The confirmer
    ///   answers them without a dialog, Full access included; only the vault's dialogs
    ///   (`vault_scope::DIALOG_METHODS`) still wait for the user's answer in the native dialog.
    ///
    /// A call that needs no confirmation (`ConfirmationNotApplicable`) gets `None` and proceeds unchanged.
    async fn own_token(
        &self,
        origin: Origin,
        method: &str,
        params: &Value,
        trusted_access_switch: bool,
    ) -> Result<Option<ConfirmationToken>, CoreError> {
        match origin {
            Origin::Ui => {
                let trusted = self.shared.config.trusted_ui_shell
                    && TRUSTED_SURFACE_METHODS.contains(&method)
                    && (method != "session/agent/update" || trusted_access_switch);
                if trusted {
                    self.trusted_token(method, params)
                } else {
                    Ok(None)
                }
            }
            Origin::DebugApi => match self
                .shared
                .gate
                .request_confirmation_as(method, params, vw_gate::Requester::DebugApi)
                .await
            {
                Ok(token) => Ok(Some(token)),
                Err(GateError::ConfirmationNotApplicable { .. }) => Ok(None),
                Err(error) => Err(CoreError::from_gate(method, &error)),
            },
        }
    }

    /// The gate's token for a call a trusted surface made. `None` when the exact call needs no
    /// confirmation (an interrupt), so it proceeds as it would without the relaxation.
    fn trusted_token(
        &self,
        method: &str,
        params: &Value,
    ) -> Result<Option<ConfirmationToken>, CoreError> {
        match self
            .shared
            .gate
            .request_trusted_confirmation(method, params)
        {
            Ok(token) => Ok(Some(token)),
            Err(GateError::ConfirmationNotApplicable { .. }) => Ok(None),
            Err(error) => Err(CoreError::from_gate(method, &error)),
        }
    }
}
