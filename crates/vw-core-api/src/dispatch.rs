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
//!   (`own_token`), except the vault, which needs a native answer. On the vault, every
//!   command first passes `vault_scope::admit`, which refuses the Debug API's user-only vault acts.
//! - The full-access alert follows a switch of a session, or a write of the default profile for new
//!   sessions, from a lower mode to Full access (`full_access_alert`); never a reopen or a start.
//! - A new registered copy's first `turn/start` receives one package setup fact after the user's text;
//!   only a successful send marks that fact delivered for this run.

use serde_json::{Value, json};
use tracing::warn;
use vw_gate::{ConfirmationToken, GateError, Reply, TRUSTED_SURFACE_METHODS};

use crate::command::{Command, CommandResult, Origin};
use crate::core::Core;
use crate::error::{CoreError, ErrorCode, clip};
use crate::event::CoreEvent;
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
        // A Stop counts before it waits for anything (`stops.rs`).
        let stopping = crate::stops::requested(&self.shared, &command);
        crate::secret_scan::note_command(&self.shared, &command);
        // A Debug API pick names its target or is refused by name, before the session lease or any native work.
        let unanswered = match &command {
            Command::AnswerCallback { callback_id, .. } => {
                crate::ask::cards::ensure_unanswered(&self.shared, callback_id)
            }
            _ => Ok(()),
        };
        let lease = match unanswered.and_then(|()| crate::preview_pick::admit(origin, &command)) {
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
        crate::stops::finished(&self.shared, &stopping);
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
        // Vibe's unified harness has no `tools/list`: answered here from the session's `runtime/read`.
        if let Command::Call { method, params, .. } = &command
            && method == crate::tools_list::METHOD
        {
            return crate::tools_list::answer(self, origin, params.clone()).await;
        }
        match plan(command) {
            Planned::Session { method, params } => self.session_call(origin, method, params).await,
            Planned::Forward {
                method,
                mut params,
                token,
                trusted_access_switch,
            } => {
                let _interruption = if method == "turn/interrupt" {
                    self.shared.agent_tools.as_ref().and_then(|tools| {
                        params
                            .get("sessionId")
                            .and_then(Value::as_str)
                            .map(|id| tools.on_demand.interrupting(id))
                    })
                } else {
                    None
                };
                let browser_start = self.start_browser_turn(&method, &params);
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
                        None,
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

    /// Tracks browser use for a submitted or queued turn before forwarding it.
    fn start_browser_turn(&self, method: &str, params: &Value) -> Option<String> {
        // The user's next message lifts a Stop of the agent's browser use.
        let browser_start = matches!(method, "turn/start" | "session/turn/enqueue")
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
        browser_start
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
    pub(crate) async fn local(
        &self,
        origin: Origin,
        command: Command,
    ) -> Result<CommandResult, CoreError> {
        match command {
            Command::GithubReads { request } => self.github_reads(request).await,
            Command::AskGrants { request } => self.ask_grants(request),
            Command::ChromeControl { request } => self.chrome_control(origin, request).await,
            Command::VibeRules { request } => self.vibe_rules(origin, request).await,
            Command::KeepAwake { request } => self.keep_awake(request).await,
            Command::NewChat { request } => self.new_chat(origin, request).await,
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
                crate::ask::cards::answer(&self.shared, origin, session_id, callback_id, output)
                    .await
            }
            Command::ListCallbacks {} => Ok(CommandResult::Callbacks {
                open: self
                    .shared
                    .gate
                    .open_callbacks()
                    .into_iter()
                    .map(CallbackInfo::from)
                    .chain(crate::ask::tool_calls::open(&self.shared))
                    .collect(),
            }),
            Command::RequestConfirmation { method, params } => {
                crate::session_ownership::check_delete(&self.shared, &method, &params).await?;
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
            Command::CopyText { text } => {
                if text.len() > 1_000_000 {
                    return Err(CoreError::new(
                        ErrorCode::InvalidParams,
                        "clipboard text exceeds one million bytes",
                    ));
                }
                self.shared
                    .config
                    .clipboard
                    .as_ref()
                    .ok_or_else(|| CoreError::new(ErrorCode::Internal, "clipboard is unavailable"))?
                    .write_text(text)
                    .await
                    .map_err(|_| CoreError::new(ErrorCode::Internal, "clipboard is unavailable"))?;
                Ok(CommandResult::Done {})
            }
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
    #[allow(clippy::too_many_lines, reason = "exhaustive typed command routing")]
    async fn feature(&self, origin: Origin, command: Command) -> Result<CommandResult, CoreError> {
        match command {
            Command::DecideProtectedChange {
                session_id,
                proposal_id,
                approve,
                note,
            } => Ok(CommandResult::ProtectedChange {
                proposal: crate::protected_changes::decide(
                    &self.shared,
                    session_id,
                    proposal_id,
                    approve,
                    note,
                    if crate::command_origin::debug_api_acts(&self.shared, origin) {
                        vw_ask::AnsweredBy::DebugApi
                    } else {
                        vw_ask::AnsweredBy::Person
                    },
                )
                .await?,
            }),
            Command::RefreshPlugins { session_id } => {
                let pending = self.refresh_plugins(&session_id).await?;
                Ok(CommandResult::Reply {
                    result: json!({"pending": pending}),
                    client_user_message_id: None,
                    user_entry_ids: Vec::new(),
                })
            }
            Command::StopSession { session_id } => crate::stop_tree::stop(self, &session_id).await,
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
            Command::Skills { request } => Ok(CommandResult::Skills {
                result: self.skills(origin, request).await?,
            }),
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
    async fn forward_with_access_switch(
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
        let _refresh = self.plugin_refresh_lock(method, &params).await?;
        // Under the chat's send lock (`stops.rs`): a Stop's interrupt reaches a turn a checked send started; a Stop
        // since the continuation was scheduled refuses it here, before a stopped chat is reopened (the test that
        // decides runs again right before the send, `turn_start::call`).
        let params = crate::stop_tree::admit(&self.shared, method, params, stop).await?;
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
        if let Some(opening) = entry_project {
            opening.finish(&self.shared, &reply.result);
        }
        if method == "session/fork" {
            crate::model_choices::forked(&self.shared, &params, &reply.result);
        }
        if matches!(method, "runtime/read" | "config/reload")
            && let Some(id) = params["sessionId"].as_str()
            && let Some(runtime) = reply.result.get("runtime")
        {
            crate::model_choices::publish(&self.shared, id, runtime.clone()).await;
        }
        if method == "turn/interrupt"
            && let Some(id) = params["sessionId"].as_str()
        {
            crate::stop_tree::descendants(&self.shared, id).await?;
        }
        crate::session_loops::after(&self.shared, method, &params, &mut reply.result).await?;
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
        // credentials in remote addresses of Vibe's git listings never leave the core (`secret_scrub`).
        reply.result = crate::secret_scrub::git_listing(method, std::mem::take(&mut reply.result));
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
        if let Some(id) = needs_channel(method, params)
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
