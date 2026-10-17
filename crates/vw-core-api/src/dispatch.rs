//! The one dispatcher: every `Command` from either client is executed here, and every one that
//! reaches Vibe goes through the method gate. The HTTP routes and the Tauri
//! commands are thin adapters over [`Core::execute`].
//!
//! Depends on: `core::Shared` (gate, store, supervisor), `sessions`, `command`, `error`, `event`,
//! `view`. Called by: vw-debug-api routes, the Tauri bridge, tests.
//! Invariants:
//! - No path to the transport except `Gate::call` and `Gate::answer_callback`.
//! - Every failure is logged with the origin and command, never with params values, and published
//!   as a `refused` event when it is a refusal (nothing was sent).
//! - Every reply of Vibe's git listings goes through `secret_scrub` (no credentials in remote addresses).
//! - The Debug API origin acts as the user: the core supplies the confirmation token for it
//!   (`own_token`), except Full access and the vault, which need a native answer. On the vault, every
//!   command first passes `vault_scope::admit`, which refuses the Debug API's user-only vault acts.
//! - After a profile switch that needed no confirmation (`session/agent/update`, or `config/write`
//!   touching `/default_agent`), `runtime/read` is called and an alert is raised when Vibe reports
//!   `bypassToolPermissions: true` (a custom profile TOML can redefine `ask`).
//! - A new registered copy's first `turn/start` receives one package setup fact after the user's text;
//!   only a successful send marks that fact delivered for this run.

use serde_json::{Value, json};
use tracing::warn;
use vw_gate::{ConfirmationToken, GateError, Reply, TRUSTED_SURFACE_METHODS};

use crate::command::{Command, CommandResult, Origin};
use crate::core::Core;
use crate::error::{CoreError, ErrorCode, clip};
use crate::event::{AlertKind, CoreEvent};
use crate::plan::{Planned, plan};
use crate::routing::needs_channel;
use crate::sessions;
use crate::view::{AnomalyReport, CallbackInfo};

impl Core {
    /// Executes one command for `origin`.
    ///
    /// # Errors
    /// [`CoreError`] with a stable code: gate refusals (`denied`, `invalidParams`,
    /// `confirmationRequired`, ...), `notFound` for sessions not loaded, `serverUnavailable`,
    /// `timeout` and `serverError` for transport failures.
    pub async fn execute(
        &self,
        origin: Origin,
        command: Command,
    ) -> Result<CommandResult, CoreError> {
        let name = command.name();
        crate::secret_scan::note_command(&self.shared, &command);
        // A Debug API pick names its target or is refused by name, before the session lease or any native work.
        let lease = match crate::preview_pick::admit(origin, &command) {
            Ok(()) => self.session_change_lease(&command).await,
            Err(error) => Err(error),
        };
        // Boxed once per command (clippy large_futures): the dispatcher's future holds the state of every command
        // kind and was at the 16 KiB limit; filing hooks in `forward` crossed it.
        let result = match lease {
            Ok(_lease) => Box::pin(self.dispatch(origin, command)).await,
            Err(error) => Err(error),
        }
        .map_err(|error| self.not_installed_reason(error));
        if let Err(error) = &result {
            self.note_failure(origin, name, error);
        }
        result
    }

    /// while Vibe is not installed every server-bound command fails at once with `serverUnavailable`;
    /// it then says why in the supervisor's words, with the reason word `notInstalled` (the transport only knows
    /// "no ready server").
    fn not_installed_reason(&self, mut error: CoreError) -> CoreError {
        if error.code == ErrorCode::ServerUnavailable
            && let vw_supervisor::SupervisorState::NotInstalled { reason } =
                &*self.shared.supervisor.state().borrow()
        {
            error.message.clone_from(reason);
            error.server_code = Some("notInstalled".to_owned());
        }
        error
    }

    /// Logs a failure with its origin and publishes it as a `refused` event when nothing was
    /// sent. `execute` calls it for every failed command; adapters call it for failures of their
    /// own (a registered UI route). Method names are clipped already (`CoreError`).
    pub fn note_failure(&self, origin: Origin, command: &str, error: &CoreError) {
        warn!(
            origin = origin.as_str(),
            command,
            method = ?error.method,
            code = error.code.as_str(),
            reason = ?clip(&error.message, 300),
            "core: command failed"
        );
        if error.is_refusal_event() {
            self.shared.emit(CoreEvent::Refused {
                origin,
                command: command.to_owned(),
                method: error.method.clone(),
                code: error.code.as_str().to_owned(),
                reason: clip(&error.message, 300),
            });
        }
    }

    async fn dispatch(&self, origin: Origin, command: Command) -> Result<CommandResult, CoreError> {
        // on the vault the Debug API has the agent's rights only; a user-only vault act it asks for (directly
        // or through a window command under its UI-control capability) is refused here, before anything runs.
        let origin = crate::vault_scope::admit(&self.shared, origin, &command).await?;
        // only `worktrees start` sets `agentConfig.worktree` (after the preflight and the branch).
        if let Some(refusal) = crate::worktrees::raw_worktree_refusal(&command) {
            return Err(refusal);
        }
        match plan(command) {
            Planned::Session { method, params } => self.session_call(origin, method, params).await,
            Planned::Forward {
                method,
                mut params,
                token,
                trusted_access_switch,
            } => {
                // The user's next message lifts a Stop of the agent's browser use.
                let browser_start =
                    matches!(method.as_str(), "turn/start" | "session/turn/enqueue")
                        .then(|| {
                            params
                                .get("sessionId")
                                .and_then(Value::as_str)
                                .map(str::to_owned)
                        })
                        .flatten();
                if let Some(id) = &browser_start {
                    crate::browser::turn_started(&self.shared, id);
                }
                let setup_id = params
                    .get("sessionId")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let setup_candidate = if method == "turn/start" {
                    if let Some(id) = &setup_id {
                        self.first_turn_setup(id).await
                    } else {
                        None
                    }
                } else {
                    None
                };
                let mut setup_guard = if setup_candidate.is_some() {
                    Some(self.shared.worktrees.setup_sent.lock().await)
                } else {
                    None
                };
                let setup_first = if let (Some(guard), Some(id)) = (&setup_guard, &setup_id)
                    && !guard.contains(id)
                {
                    setup_candidate
                } else {
                    None
                };
                if let Some(Some(line)) = &setup_first
                    && let Some(message) = params.get_mut("message").and_then(Value::as_array_mut)
                {
                    message.push(json!({"type": "text", "text": line}));
                }
                let reply = match self
                    .forward_with_access_switch(
                        origin,
                        &method,
                        params,
                        token,
                        trusted_access_switch,
                    )
                    .await
                {
                    Ok(reply) => reply,
                    Err(error) => {
                        if let Some(id) = &browser_start {
                            self.release_failed_browser_turn(id).await;
                        }
                        return Err(error);
                    }
                };
                if setup_first.is_some()
                    && let (Some(guard), Some(id)) = (&mut setup_guard, setup_id)
                {
                    guard.insert(id);
                }
                Ok(CommandResult::Reply {
                    result: reply.result,
                    client_user_message_id: reply.client_user_message_id,
                    user_entry_ids: reply.user_entry_ids,
                })
            }
            // Boxed (clippy large_futures): the local commands' state exceeds 16 KiB when
            // project commands join the other local commands.
            Planned::Local(command) => Box::pin(self.local(origin, command)).await,
        }
    }

    /// Release browser use after a failed send when no turn is active.
    async fn release_failed_browser_turn(&self, id: &str) {
        if let Some(hub) = &self.shared.preview {
            hub.service.browser_turn_observed(id);
            let active = self.shared.store.view(id).is_some_and(|view| {
                view.state["turns"]
                    .as_array()
                    .is_some_and(|turns| turns.iter().any(|turn| turn["status"] == "in_progress"))
            });
            if !active && let Some(epoch) = hub.service.browser_turn_epoch(id) {
                let _ = hub.service.release_browser_for_turn(id, epoch).await;
            }
        }
    }

    /// Commands the core answers itself, or with a dedicated gate entry point.
    async fn local(&self, origin: Origin, command: Command) -> Result<CommandResult, CoreError> {
        match command {
            Command::SecretScan { request } => self.secret_scan(request).await,
            Command::ListSessions {
                limit,
                cursor,
                include_archived,
            } => {
                self.list_sessions(origin, limit, cursor, include_archived)
                    .await
            }
            Command::ReadSession { session_id } => Ok(CommandResult::Session {
                snapshot: sessions::snapshot(&self.shared, &session_id)?,
                detail: Value::Null,
            }),
            Command::AnswerCallback {
                session_id,
                callback_id,
                output,
            } => {
                // No channel step: a callback lives only on its session's open channel (a closed or
                // crashed channel ends it), so an answer for a session without one is refused by the
                // gate's callback book as unknown.
                let output = self.shared.connectors.callback_answer(
                    &session_id,
                    &callback_id,
                    &self.shared.gate,
                    output,
                )?;
                let answer = output.clone();
                let result = self
                    .shared
                    .gate
                    .answer_callback(&session_id, &callback_id, output)
                    .await
                    .map_err(|e| CoreError::from_gate("callback/result", &e))?;
                // The user's yes on Vibe's card counts for the guard inside our own tool (one ask per call).
                if let Some(ledger) = crate::one_ask::of(&self.shared) {
                    ledger.card_answered(&session_id, &callback_id, &answer, origin);
                }
                self.shared.emit(CoreEvent::CallbackAnswered {
                    session_id,
                    callback_id,
                    origin,
                });
                Ok(CommandResult::Reply {
                    result,
                    client_user_message_id: None,
                    user_entry_ids: Vec::new(),
                })
            }
            Command::ListCallbacks {} => Ok(CommandResult::Callbacks {
                open: self
                    .shared
                    .gate
                    .open_callbacks()
                    .into_iter()
                    .map(CallbackInfo::from)
                    .collect(),
            }),
            Command::RequestConfirmation { method, params } => {
                let token = self
                    .shared
                    .gate
                    .request_confirmation_as(&method, &params, crate::command::requester(origin))
                    .await
                    .map_err(|e| CoreError::from_gate(&method, &e))?;
                Ok(CommandResult::Confirmation {
                    token: token.as_str().to_owned(),
                })
            }
            Command::ViewSession { session_id } => self.view_session(origin, session_id).await,
            Command::RetryReopen { session_id } => self.retry_reopen(origin, &session_id).await,
            Command::CancelReopen { session_id } => Ok(self.cancel_reopen(&session_id)),
            Command::ContinueInNewSession { session_id } => {
                Box::pin(self.continue_in_new(origin, &session_id)).await
            }
            Command::ReadStatus {} => Ok(CommandResult::Status {
                status: self.status(),
            }),
            Command::ReadAnomalies { session_id } => self.anomalies(session_id),
            Command::Keys { request } => crate::key_replace::execute(&self.shared, request).await,
            Command::Vibe { request } => {
                crate::vibe_install::execute(
                    self.shared.config.vibe_install.as_ref(),
                    origin,
                    request,
                )
                .await
            }
            other => self.feature(origin, other).await,
        }
    }

    /// The feature tabs' commands (terminals, instructions, documents, speech, Changes, preview): each has its
    /// own handler module; split out so `local` stays one screen long.
    async fn feature(&self, origin: Origin, command: Command) -> Result<CommandResult, CoreError> {
        match command {
            Command::StopSession { session_id } => {
                sessions::snapshot(&self.shared, &session_id)?;
                self.shared.channels.close(&session_id).await;
                Ok(CommandResult::Reply {
                    result: json!({"stopped": true}),
                    client_user_message_id: None,
                    user_entry_ids: Vec::new(),
                })
            }
            Command::SetDefaultModel { model_alias } => {
                crate::default_model::set(&self.shared, &model_alias).await?;
                Ok(CommandResult::Reply {
                    result: json!({"modelAlias": model_alias}),
                    client_user_message_id: None,
                    user_entry_ids: Vec::new(),
                })
            }
            Command::Terminal { request } => self.terminal(request).await,
            Command::Instructions { request } => self.instructions(origin, request).await,
            Command::ProjectInstructions { request } => {
                self.project_instructions(origin, request).await
            }
            Command::Documents { request } => {
                crate::documents::execute(&self.shared, request).await
            }
            Command::Speech { request } => self.speech(origin, request).await,
            Command::Libraries { request } => crate::work::libraries(&self.shared, request).await,
            Command::Images { request } => Ok(crate::work::images(&self.shared, &request)),
            Command::Changes { request } => Ok(CommandResult::Changes {
                result: self.changes(origin, request).await?,
            }),
            Command::LineComments { request } => self.line_comments(request).await,
            Command::Preview { request } => self.preview(request).await,
            Command::Browser { request } => self.browser(origin, request).await,
            Command::Pages { request } => self.pages(origin, request).await,
            Command::Projects { request } => self.projects(origin, request).await,
            Command::Files { request } => self.files(origin, request).await,
            Command::Connectors { request } => self.connectors(origin, request).await,
            Command::Marketplace { request } => self.marketplace(origin, request).await,
            Command::Worktrees { request } => Ok(CommandResult::Worktrees {
                result: self.worktrees(origin, request).await?,
            }),
            Command::GitBrowser { request } => Ok(CommandResult::GitBrowser {
                result: self.git_browser(request).await?,
            }),
            Command::Git { request } => Ok(CommandResult::Git {
                result: self.git(origin, request).await?,
            }),
            Command::Processes { request } => Ok(CommandResult::Processes {
                result: self.processes(origin, request).await?,
            }),
            Command::Secrets { request } => Ok(CommandResult::Secrets {
                result: crate::secrets::execute(&self.shared, request).await?,
            }),
            Command::Safety { request } => Ok(CommandResult::Safety {
                result: crate::safety::execute(&self.shared, request)?,
            }),
            Command::Network { request } => Ok(CommandResult::Network {
                result: self.network(origin, request).await?,
            }),
            Command::PickFolder { path } => self.pick_folder(origin, &path),
            Command::SessionTools { request } => Ok(CommandResult::SessionTools {
                result: self.session_tools(origin, request).await?,
            }),
            Command::Search { request } => Ok(CommandResult::Search {
                result: self.search(request).await?,
            }),
            other => Err(CoreError::new(
                ErrorCode::Internal,
                format!("{} has no local handler", other.name()),
            )),
        }
    }

    /// `pickFolder`: the Debug API's "Choose folder". The window is refused (it uses the native
    /// dialog, see `Command::PickFolder`).
    fn pick_folder(&self, origin: Origin, path: &str) -> Result<CommandResult, CoreError> {
        if origin != Origin::DebugApi {
            return Err(CoreError::new(
                ErrorCode::Denied,
                "the window chooses folders in the native dialog (Choose folder)",
            ));
        }
        self.shared
            .gate
            .register_picked_folder(std::path::Path::new(path))
            .map_err(|error| {
                CoreError::new(
                    ErrorCode::InvalidParams,
                    format!("cannot use that folder: {error}"),
                )
            })?;
        Ok(CommandResult::Done {})
    }

    fn anomalies(&self, session_id: String) -> Result<CommandResult, CoreError> {
        let anomalies = self
            .shared
            .store
            .anomalies(&session_id)
            .ok_or_else(|| CoreError::not_loaded(&session_id))?;
        let counts = anomalies.counts();
        Ok(CommandResult::Anomalies {
            report: AnomalyReport {
                session_id,
                counts: counts.into(),
                significant: counts.significant(),
                recent: anomalies.recent().into_iter().map(Into::into).collect(),
            },
        })
    }

    /// A call whose result is a session state: forwarded, loaded, answered with the snapshot.
    pub(crate) async fn session_call(
        &self,
        origin: Origin,
        method: &str,
        params: Value,
    ) -> Result<CommandResult, CoreError> {
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
        self.forward_with_access_switch(origin, method, params, token, false)
            .await
    }

    /// The typed composer switch is the sole route that can use the trusted-window Full access
    /// exception. A raw `call` of the same method must still bring a native confirmation token.
    async fn forward_with_access_switch(
        &self,
        origin: Origin,
        method: &str,
        params: Value,
        token: Option<String>,
        trusted_access_switch: bool,
    ) -> Result<Reply, CoreError> {
        // A composer's runtime read must wait for the open's verified mode too.
        let _runtime_restore = self.runtime_restore_lock(method, &params).await;
        // A stuck session is never opened implicitly (`stuck.rs`): refused at once, nothing sent.
        let opens = needs_channel(method, &params).or_else(|| {
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
        if token.is_none() {
            token = self
                .own_token(origin, method, &params, trusted_access_switch)
                .await?;
        }
        if let Some(id) = needs_channel(method, &params)
            && !self.shared.channels.has_channel(id)
        {
            // Refusals first: a call the gate refuses must not open (and lease) the session.
            self.shared
                .gate
                .check(method, &params, token.as_ref())
                .map_err(|e| CoreError::from_gate(method, &e))?;
            let id = id.to_owned();
            self.bounded_open(&id, self.attach_session(origin, &id))
                .await?;
        }
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
        self.shared
            .gate
            .check(method, &params, token.as_ref())
            .map_err(|e| CoreError::from_gate(method, &e))?;
        crate::attachment_save::prepare(&self.shared, method, &mut params).await?;
        let compaction_channel = crate::dispatch_compaction::channel(&self.shared, method, &params);
        let gate = &self.shared.gate;
        let call = gate.call(method, params.clone(), token.as_ref());
        let reply = match restore.as_ref().and_then(|_| params["sessionId"].as_str()) {
            // A resume is an open too: bounded, and a session Vibe cannot reopen is marked stuck.
            Some(id) => {
                let id = id.to_owned();
                let mut reply = None;
                self.bounded_open(&id, async {
                    reply = Some(call.await.map_err(|e| CoreError::from_gate(method, &e))?);
                    Ok(())
                })
                .await?;
                reply
                    .ok_or_else(|| CoreError::new(ErrorCode::Internal, "resume without a reply"))?
            }
            None => {
                match crate::session_end::missing_reply(&self.shared, method, &params, call.await) {
                    Ok(reply) => reply,
                    Err(error) => {
                        return Err(crate::dispatch_compaction::failed(
                            &self.shared,
                            method,
                            &params,
                            compaction_channel,
                            &error,
                        )
                        .await);
                    }
                }
            }
        };
        if let Some(opening) = entry_project {
            opening.finish(&self.shared, &reply.result);
        }
        // A delete ends the session here, in one place (`session_end.rs`).
        sessions::observe(&self.shared, method, &params, &reply.result).await?;
        if let Some(restore) = restore {
            self.restore_profile(origin, restore).await?;
        }
        if token.is_none() && switches_profile(method, &params) {
            self.check_bypass(origin, &params).await;
        }
        // credentials in remote addresses of Vibe's git listings never leave the core (`secret_scrub`).
        let mut reply = reply;
        reply.result = crate::secret_scrub::git_listing(method, std::mem::take(&mut reply.result));
        Ok(reply)
    }

    /// The confirmation token the core supplies itself for a call that came without one, or `None`:
    /// - the window (trusted surface): for `TRUSTED_SURFACE_METHODS` only, without a dialog;
    /// - the Debug API (it acts as the user): for calls that need a confirmation; the
    ///   confirmer answers most requests directly, while Full access still opens a native dialog.
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

    /// Reads the runtime after an unconfirmed profile switch and raises an alert when tool
    /// permissions are bypassed. A failed read is logged, not raised: the switch itself worked.
    pub(crate) async fn check_bypass(&self, origin: Origin, params: &Value) {
        let Some(session_id) = params.get("sessionId").and_then(Value::as_str) else {
            return;
        };
        let read = self
            .shared
            .gate
            .call("runtime/read", json!({"sessionId": session_id}), None)
            .await;
        match read {
            Ok(reply) => {
                let bypass = reply
                    .result
                    .pointer("/runtime/bypassToolPermissions")
                    .and_then(Value::as_bool)
                    == Some(true);
                if bypass {
                    warn!(
                        origin = origin.as_str(),
                        session_id, "profile switch bypasses tool permissions"
                    );
                    self.shared.emit(CoreEvent::Alert {
                        session_id: Some(session_id.to_owned()),
                        kind: AlertKind::BypassToolPermissions,
                        message: "This access mode runs every tool without asking (its profile turns tool permissions off).".to_owned(),
                    });
                }
            }
            Err(error) => {
                warn!(session_id, %error, "runtime/read after a profile switch failed");
            }
        }
    }
}

/// True for calls that change which profile a session (or new sessions) use.
pub(crate) fn switches_profile(method: &str, params: &Value) -> bool {
    match method {
        "session/agent/update" => true,
        "config/write" => params
            .get("ops")
            .and_then(Value::as_array)
            .is_some_and(|ops| {
                ops.iter()
                    .any(|op| op.get("path").and_then(Value::as_str) == Some("/default_agent"))
            }),
        _ => false,
    }
}

/// Maps a request body that is not a valid command (vw-debug-api uses it for decode errors).
///
/// The text names keys only, never values: serde's messages can quote the caller's values
/// (`invalid type: string "..."`), and refusals are logged and broadcast. Messages about a
/// missing or unknown field, or an unknown command type, name our own schema's keys (or the
/// caller's key, clipped) and are kept; anything else becomes a fixed text.
#[must_use]
pub fn invalid_command(error: &serde_json::Error) -> CoreError {
    let text = error.to_string();
    let keep = ["missing field `", "unknown field `", "unknown variant `"]
        .iter()
        .any(|prefix| text.starts_with(prefix));
    let detail = if keep {
        // Drop serde's "expected one of ..." tail and the position: the key is what matters.
        let head = text.split(", expected").next().unwrap_or(&text);
        clip(head.split(" at line ").next().unwrap_or(head), 200)
    } else if error.is_syntax() || error.is_eof() {
        "the body is not valid JSON".to_owned()
    } else {
        "a field has the wrong type or value".to_owned()
    };
    CoreError::new(
        ErrorCode::InvalidCommand,
        format!("not a valid command: {detail}"),
    )
}
