//! The agent browser in the core: the `browser` command for the window and the Debug API, the
//! `browser_read` / `browser_act` agent tools' shared paths, the navigation the driver asks for (the session's port
//! rules and the network guard: `previews::open_for_session`), Stop / Resume, and the reset of Stop at the session's
//! next turn.
//!
//! Depends on: vw-browser (`Browser`, `Sessions`), `previews` (hub, open path), `browser_view`, `core::Shared`.
//! Called by: `dispatch` (`Command::Browser`, `turn/start`), `agent_tools::browser_tools`, `previews` (events,
//! session end).
//! Invariants:
//! - Browser navigation accepts local and public addresses under the same URL policy. Agent calls retain
//!   refusal reporting and pinned App preview port ownership checks; the window and Debug API act as the user.
//! - Stop applies to the agent only; the session's next `turn/start` (the user's next message) lifts it.
//! - Vault choices and saves are window-only; the vault scope refuses Debug API calls before page access.

use std::sync::Arc;

use vw_browser::annotate;
use vw_browser::{ActError, Actor, Browser, NavFuture, Navigate, Outcome, ReadOptions};

use crate::browser_view::{BrowserRequest, BrowserResult, BrowserShot};
use crate::command::{CommandResult, Origin};
use crate::core::{Core, Shared};
use crate::error::{CoreError, ErrorCode};
use crate::previews::{hub, open_for_session};

/// Opens URLs for the driver: the session's ports and owners, the policy, the guard.
pub(crate) struct CoreNav {
    pub(crate) shared: Arc<Shared>,
    /// Identifies agent calls for refusal reporting and pinned App preview port ownership checks.
    pub(crate) strict: bool,
}

impl Navigate for CoreNav {
    fn open<'a>(&'a self, session_id: &'a str, url: &'a str) -> NavFuture<'a> {
        Box::pin(async move {
            let url = with_scheme(url);
            open_for_session(
                &self.shared,
                session_id,
                (Some(url), None, None),
                self.strict,
            )
            .await
            .map(|_| ())
            .map_err(|e| e.message)
        })
    }
}

/// `example.com/x` -> `https://example.com/x`, `localhost:5173` -> `http://localhost:5173` (what the address bar and
/// the agent often write). Anything that already names a scheme (`https:`, `file:`, `javascript:`) is left for the
/// policy to judge.
fn with_scheme(url: &str) -> String {
    let url = url.trim();
    let named_scheme = url.split_once(':').is_some_and(|(scheme, rest)| {
        !scheme.is_empty()
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
            && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
            && !rest.starts_with(|c: char| c.is_ascii_digit())
    });
    if named_scheme {
        url.to_owned()
    } else if url.starts_with("localhost")
        || url.starts_with("127.0.0.1")
        || url.starts_with("[::1]")
    {
        format!("http://{url}")
    } else {
        format!("https://{url}")
    }
}

/// A `browser` `read` for the user or the Debug API: the observation and an optional screenshot.
async fn read_result(
    shared: &Shared,
    driver: &Browser,
    session_id: &str,
    options: ReadOptions,
    screenshot: Option<bool>,
    origin: Origin,
) -> Result<BrowserResult, String> {
    let budget = options.max_bytes;
    let (mut observation, shot) = driver
        .read(
            session_id,
            options,
            screenshot.unwrap_or(false),
            actor(shared, origin),
        )
        .await?;
    crate::agent_tools::attach_tab(shared, session_id, &mut observation);
    vw_browser::observe::fit(
        &mut observation,
        budget.clamp(
            vw_browser::observe::MIN_BUDGET,
            vw_browser::observe::MAX_BUDGET,
        ),
    );
    Ok(BrowserResult::Observation {
        observation,
        screenshot: shot.map(|s| BrowserShot {
            png_base64: s.base64(),
            width: s.width,
            height: s.height,
        }),
    })
}

/// Parses the typed read fields away from the browser dispatcher.
async fn read_request(
    shared: &Shared,
    driver: &Browser,
    origin: Origin,
    request: BrowserRequest,
) -> Result<BrowserResult, String> {
    let BrowserRequest::Read {
        session_id,
        tab_id,
        max_refs,
        text_page,
        max_bytes,
        find,
        screenshot,
    } = request
    else {
        return Err("unexpected browser read request".to_owned());
    };
    let options = read_options(
        max_refs.map(u64::from),
        text_page.map(u64::from),
        max_bytes.map(u64::from),
        find.as_deref(),
    );
    let page = crate::browser_tabs::user_page(
        &hub(shared).map_err(|error| error.message)?.service,
        &session_id,
        tab_id.as_deref(),
    )
    .map_err(|error| error.message)?;
    crate::browser_tabs::front_page(
        &hub(shared).map_err(|error| error.message)?.service,
        &session_id,
        &page,
    )
    .await
    .map_err(|error| error.message)?;
    read_result(shared, driver, &page, options, screenshot, origin).await
}

/// Who runs a `browser` command: the Debug API (also for the window's commands while it drives the window, so its
/// actions keep the Debug API's checks and log name), or the user (the window).
fn actor(shared: &Shared, origin: Origin) -> Actor {
    if crate::command_origin::debug_api_acts(shared, origin) {
        Actor::DebugApi
    } else {
        Actor::User
    }
}

/// The strip and log span every page of the session; Stop belongs to the owner session.
fn browser_state(shared: &Shared, owner: &str) -> Result<vw_browser::BrowserState, CoreError> {
    let hub = hub(shared)?;
    let current = hub.service.agent_page(owner);
    let mut state = hub.browser.sessions().state(owner);
    state.window_id = current
        .as_deref()
        .and_then(|page| hub.service.focusable_window_id(page));
    for page in hub.service.web_tabs(owner) {
        let part = hub.browser.sessions().state(&page);
        state.agent_active |= part.agent_active;
        state.in_flight |= part.in_flight;
        state.stopped |= part.stopped;
        if state.pending_native_click.is_none()
            || (current.as_deref() == Some(page.as_str()) && part.pending_native_click.is_some())
        {
            state
                .pending_native_click
                .clone_from(&part.pending_native_click);
        }
        state.last_agent_at_ms = state.last_agent_at_ms.max(part.last_agent_at_ms);
        state.log.extend(part.log);
        for mut receipt in part.receipts {
            owner.clone_into(&mut receipt.session_id);
            if !state.receipts.contains(&receipt) {
                state.receipts.push(receipt);
            }
        }
    }
    state.log.sort_by_key(|entry| entry.at_ms);
    if state.log.len() > vw_browser::LOG_CAP {
        state.log.drain(..state.log.len() - vw_browser::LOG_CAP);
    }
    for (index, entry) in state.log.iter_mut().enumerate() {
        entry.seq = u32::try_from(index + 1).unwrap_or(u32::MAX);
    }
    state.receipts.sort_by_key(|receipt| receipt.at_ms);
    Ok(state)
}

/// A `browser` `capture` for the window or the Debug API.
async fn capture_result(
    shared: &std::sync::Arc<Shared>,
    session_id: &str,
    args: &serde_json::Value,
    origin: Origin,
) -> Result<BrowserResult, CoreError> {
    let owner = hub(shared)?
        .service
        .owner_session_id(session_id)
        .ok_or_else(|| CoreError::new(ErrorCode::Denied, "browser page has no session owner"))?;
    if shared.store.view(&owner).is_none() {
        return Err(CoreError::not_loaded(&owner));
    }
    let mut capture = crate::browser_capture::run(shared, session_id, args, actor(shared, origin))
        .await
        .map_err(|e| act_error(&e))?;
    crate::agent_tools::attach_tab(shared, session_id, &mut capture);
    Ok(BrowserResult::Capture { capture })
}

/// The browser of this run.
pub(crate) fn browser(shared: &Shared) -> Result<&Browser, CoreError> {
    Ok(&hub(shared)?.browser)
}

/// An act error as a core error (refusals and Stop: `denied`).
pub(crate) fn act_error(error: &ActError) -> CoreError {
    let code = match error.outcome {
        Outcome::Failed => ErrorCode::Internal,
        _ => ErrorCode::Denied,
    };
    let mut result = CoreError::new(code, error.text.clone());
    result.capture_block = error.capture_block;
    result
}

/// The read options from optional numbers.
pub(crate) fn read_options(
    max_refs: Option<u64>,
    text_page: Option<u64>,
    max_bytes: Option<u64>,
    find: Option<&str>,
) -> ReadOptions {
    let mut options = ReadOptions::default();
    if let Some(n) = max_refs.and_then(|n| usize::try_from(n).ok()) {
        options.max_refs = n;
    }
    if let Some(n) = text_page.and_then(|n| usize::try_from(n).ok()) {
        options.text_page = Some(n.min(10_000));
    }
    if let Some(n) = max_bytes.and_then(|n| usize::try_from(n).ok()) {
        options.max_bytes = n;
    }
    options.find = find
        .map(str::trim)
        .filter(|q| !q.is_empty())
        .map(|q| q.chars().take(100).collect());
    options
}

/// The user's next message lifts the Stop of that session.
pub(crate) fn turn_started(shared: &Shared, session_id: &str) {
    shared.asks.clear_question_holds(Some(session_id));
    if let Some(hub) = &shared.preview {
        hub.browser.sessions().cancel_actions(session_id);
        hub.service.browser_turn_started(session_id);
        hub.browser.sessions().resume(session_id);
        for key in hub.service.web_tabs(session_id) {
            hub.browser.sessions().cancel_actions(&key);
            hub.browser.sessions().resume(&key);
        }
    }
}

/// Chat Stop also fences built-in browser work and releases its activity without closing the document.
pub(crate) fn stop_for_chat(shared: &Shared, session_id: &str) {
    if let Some(hub) = &shared.preview {
        for page in std::iter::once(session_id.to_owned()).chain(hub.service.web_tabs(session_id)) {
            hub.browser.sessions().stop(&page);
            hub.browser.sessions().cancel_confirmations(&page);
        }
    }
}

/// A completed turn removes eviction protection while keeping the browser document alive.
pub(crate) async fn turn_ended(shared: &Shared, session_id: &str, epoch: u64) {
    let active = shared.store.view(session_id).is_some_and(|view| {
        view.state["turns"]
            .as_array()
            .is_some_and(|turns| turns.iter().any(|turn| turn["status"] == "in_progress"))
    });
    if active {
        return;
    }
    if let Some(hub) = &shared.preview {
        // The chat owns the turn generation. Tabs can be created mid-turn and have no matching counter.
        match hub
            .service
            .release_browser_for_turn(session_id, epoch)
            .await
        {
            Ok(true) if hub.service.browser_turn_epoch(session_id) == Some(epoch) => {
                let pages =
                    std::iter::once(session_id.to_owned()).chain(hub.service.web_tabs(session_id));
                for page in pages {
                    hub.browser.sessions().cancel_confirmations(&page);
                }
            }
            Ok(_) => {}
            Err(error) => {
                tracing::warn!(%error, session_id, "browser turn completion failed");
            }
        }
    }
}

impl Core {
    /// Executes one browser request (every origin runs it directly).
    #[allow(clippy::too_many_lines)]
    pub(crate) async fn browser(
        &self,
        origin: Origin,
        request: BrowserRequest,
    ) -> Result<CommandResult, CoreError> {
        let shared = &self.shared;
        if let Some(result) = crate::browser_settings::run(shared, &request) {
            return result.map(|result| CommandResult::Browser { result });
        }
        if let Some(result) = crate::browser_accounts::run(shared, &request).await {
            return result.map(|result| CommandResult::Browser { result });
        }
        crate::browser_accounts::admit(&request)?;
        if let Some(result) = crate::browser_tabs::run(shared, &request).await {
            return result.map(|result| CommandResult::Browser { result });
        }
        let driver = browser(shared)?;
        let denied = |text: String| CoreError::new(ErrorCode::Denied, text);
        let result = match request {
            BrowserRequest::Share { page_key } => {
                if crate::command_origin::debug_api_acts(shared, origin) {
                    return Err(denied(
                        "Sharing a page requires the user. Ask the user to use the Browser pane's share action."
                            .into(),
                    ));
                }
                crate::chrome_control::builtin_share::queue(shared, &page_key, false).await?;
                BrowserResult::Done {}
            }
            BrowserRequest::Permissions {} => crate::browser_delegation::settings(shared, None)?,
            BrowserRequest::SetPermissions { permissions } => {
                // Legacy compatibility request: returns empty permissions without writing a settings file.
                let owned = shared.clone();
                tokio::task::spawn_blocking(move || {
                    crate::browser_delegation::settings(&owned, Some(permissions))
                })
                .await
                .map_err(|join| CoreError::new(ErrorCode::Internal, join.to_string()))??
            }
            BrowserRequest::Shields {} | BrowserRequest::SetShields { .. } => {
                crate::browser_shields::run(shared, &request).await?
            }
            BrowserRequest::EnginePool {} | BrowserRequest::SetEnginePool { .. } => {
                pool_result(shared, &request).await?
            }
            BrowserRequest::Preferences {}
            | BrowserRequest::SetComposerBrowser { .. }
            | BrowserRequest::SetBuiltinEnabled { .. }
            | BrowserRequest::Accounts {}
            | BrowserRequest::SignIn { .. }
            | BrowserRequest::SignOut { .. }
            | BrowserRequest::AccountsLayout { .. }
            | BrowserRequest::CloseSignIn {} => {
                return Err(crate::browser_accounts::unhandled());
            }
            read @ BrowserRequest::Read { .. } => read_request(shared, driver, origin, read)
                .await
                .map_err(denied)?,
            BrowserRequest::Act {
                session_id,
                tab_id,
                args,
            } => {
                shared.browser_dialogs.check().map_err(denied)?;
                if !crate::new_chat::known_owner(shared, &session_id) {
                    return Err(CoreError::not_loaded(&session_id));
                }
                let page = crate::browser_tabs::user_page(
                    &hub(shared)?.service,
                    &session_id,
                    tab_id.as_deref(),
                )?;
                crate::browser_tabs::front_page(&hub(shared)?.service, &session_id, &page).await?;
                if args.get("action").and_then(serde_json::Value::as_str) == Some("fill") {
                    return fill_act(shared, driver, origin, &page, &args).await;
                }
                act_receipt(shared, driver, origin, &page, &args).await?
            }
            BrowserRequest::Capture {
                session_id,
                tab_id,
                args,
            } => {
                let page = crate::browser_tabs::user_page(
                    &hub(shared)?.service,
                    &session_id,
                    tab_id.as_deref(),
                )?;
                crate::browser_tabs::front_page(&hub(shared)?.service, &session_id, &page).await?;
                capture_result(shared, &page, &args, origin).await?
            }
            BrowserRequest::State { session_id } => BrowserResult::State {
                state: browser_state(shared, &session_id)?,
            },
            BrowserRequest::Stop { session_id } => {
                driver.sessions().stop(&session_id);
                for key in hub(shared)?.service.web_tabs(&session_id) {
                    driver.sessions().stop(&key);
                    hub(shared)?.service.dismiss_dialog(&key).await;
                }
                BrowserResult::State {
                    state: browser_state(shared, &session_id)?,
                }
            }
            BrowserRequest::Resume { session_id } => {
                driver.sessions().resume(&session_id);
                for key in hub(shared)?.service.web_tabs(&session_id) {
                    driver.sessions().resume(&key);
                }
                BrowserResult::State {
                    state: browser_state(shared, &session_id)?,
                }
            }
            BrowserRequest::Annotate {
                session_id,
                mode,
                target,
            } => annotate_request(shared, &session_id, mode.as_deref(), target.as_ref()).await?,
            BrowserRequest::CheckAnnotations {
                session_id,
                items,
                reload,
            } => BrowserResult::AnnotationStates {
                states: annotate::recheck(&hub(shared)?.service, &session_id, &items, reload)
                    .await
                    .map_err(denied)?,
            },
            BrowserRequest::CaptureSecret { .. } => {
                return Err(denied(
                    "saving a key is a user action in the browser panel".to_owned(),
                ));
            }
            BrowserRequest::ChooseSecret {
                session_id,
                tab_id,
                mode,
            } => {
                let page = crate::browser_tabs::user_page(
                    &hub(shared)?.service,
                    &session_id,
                    tab_id.as_deref(),
                )?;
                crate::browser_tabs::front_page(&hub(shared)?.service, &session_id, &page).await?;
                crate::browser_secret::choose(shared, driver, &page, &mode).await?
            }
            BrowserRequest::SaveSecret {
                session_id,
                tab_id,
                handle,
                name,
            } => {
                let page = crate::browser_tabs::user_page(
                    &hub(shared)?.service,
                    &session_id,
                    tab_id.as_deref(),
                )?;
                crate::browser_secret::save(shared, driver, &page, &handle, &name).await?
            }
            BrowserRequest::CancelSecret {
                session_id,
                tab_id,
                handle,
            } => {
                let page = crate::browser_tabs::user_page(
                    &hub(shared)?.service,
                    &session_id,
                    tab_id.as_deref(),
                )?;
                driver.cancel_secret_choice(&page, &handle);
                BrowserResult::Done {}
            }
            BrowserRequest::CancelAnnotate { session_id } => {
                cancel_annotations(shared, &session_id).await?
            }
            BrowserRequest::Tabs { .. }
            | BrowserRequest::OpenTab { .. }
            | BrowserRequest::SwitchTab { .. }
            | BrowserRequest::CloseTab { .. }
            | BrowserRequest::ReorderTabs { .. }
            | BrowserRequest::ReviewHandoff { .. }
            | BrowserRequest::HandoffTab { .. }
            | BrowserRequest::TakeBackTab { .. } => {
                return Err(CoreError::new(
                    ErrorCode::Internal,
                    "tab request escaped its handler",
                ));
            }
        };
        Ok(CommandResult::Browser { result })
    }
}

/// Removes the session's annotation overlay when its engine is mounted.
async fn cancel_annotations(shared: &Shared, session_id: &str) -> Result<BrowserResult, CoreError> {
    let service = &hub(shared)?.service;
    if let Some(page) = service.page_accessor(session_id) {
        annotate::cancel(service, &page, session_id).await;
    }
    Ok(BrowserResult::Done {})
}

/// Reads or changes the browser allowance through the service's persisted settings boundary.
async fn pool_result(
    shared: &Shared,
    request: &BrowserRequest,
) -> Result<BrowserResult, CoreError> {
    let service = &hub(shared)?.service;
    if let BrowserRequest::SetEnginePool {
        configured_parallel_sessions,
    } = request
    {
        let pool = service
            .set_engine_pool(configured_parallel_sessions)
            .await
            .map_err(|error| crate::previews::core_error(&error))?;
        shared.emit(crate::event::CoreEvent::BrowserChanged { session_id: None });
        return Ok(BrowserResult::EnginePool { pool });
    }
    Ok(BrowserResult::EnginePool {
        pool: service.engine_pool(),
    })
}

/// Runs an ordinary browser action with the caller's rights and the session's navigation policy.
async fn act_receipt(
    shared: &Arc<Shared>,
    driver: &Browser,
    origin: Origin,
    session_id: &str,
    args: &serde_json::Value,
) -> Result<BrowserResult, CoreError> {
    if args["action"] == "auth.basic" {
        let receipt = crate::browser_auth::prepare(
            shared,
            session_id,
            args,
            actor(shared, origin),
            crate::command::requester(origin),
        )
        .await
        .map_err(|e| CoreError::new(ErrorCode::Denied, e))?
        .await
        .map_err(|e| CoreError::new(ErrorCode::Denied, e))?;
        return Ok(BrowserResult::Receipt { receipt });
    }
    if matches!(
        args["action"].as_str(),
        Some("clipboard.read" | "clipboard.write" | "clipboard.read_rich" | "clipboard.write_rich")
    ) {
        let receipt = crate::browser_clipboard::run(shared, args)
            .await
            .map_err(|e| CoreError::new(ErrorCode::Denied, e))?;
        return Ok(BrowserResult::Receipt { receipt });
    }
    let action =
        vw_browser::parse(args).map_err(|error| CoreError::new(ErrorCode::InvalidParams, error))?;
    // An upload reads its files from the chat that owns this page.
    let owner = crate::previews::hub(shared)
        .ok()
        .and_then(|hub| hub.service.owner_session_id(session_id))
        .unwrap_or_else(|| session_id.to_owned());
    let action = crate::browser_upload::load(shared, &owner, action)
        .await
        .map_err(|error| CoreError::new(ErrorCode::InvalidParams, error))?;
    let nav = CoreNav {
        shared: shared.clone(),
        strict: false,
    };
    let mut receipt = driver
        .act(session_id, action, actor(shared, origin), &nav)
        .await
        .map_err(|error| act_error(&error))?;
    crate::browser_recording::save(shared, &owner, &mut receipt)
        .map_err(|message| CoreError::new(ErrorCode::Internal, message))?;
    crate::agent_tools::attach_tab(shared, session_id, &mut receipt);
    vw_browser::observe::fit(&mut receipt, vw_browser::observe::READ_BUDGET);
    Ok(BrowserResult::Receipt { receipt })
}

/// `act` `fill` takes the agent's path with the agent's rights on the vault: the native dialog
/// decides, never this request.
async fn fill_act(
    shared: &Arc<Shared>,
    driver: &Browser,
    origin: Origin,
    session_id: &str,
    args: &serde_json::Value,
) -> Result<CommandResult, CoreError> {
    let mut receipt = crate::browser_fill::fill(
        shared,
        driver,
        session_id,
        args,
        actor(shared, origin),
        crate::command::requester(origin),
    )
    .await
    .map_err(|e| CoreError::new(ErrorCode::Denied, e))?;
    crate::agent_tools::attach_tab(shared, session_id, &mut receipt);
    vw_browser::observe::fit(&mut receipt, vw_browser::observe::READ_BUDGET);
    Ok(CommandResult::Browser {
        result: BrowserResult::Receipt { receipt },
    })
}

/// "Annotate this page", out of `browser` to keep it short.
async fn annotate_request(
    shared: &Shared,
    session_id: &str,
    mode: Option<&str>,
    target: Option<&crate::preview_view::PickTarget>,
) -> Result<BrowserResult, CoreError> {
    let denied = |text: String| CoreError::new(ErrorCode::Denied, text);
    let service = &hub(shared)?.service;
    // annotation is for the app preview (the session's own dev server) only.
    if service.page_class(session_id) != Some(vw_preview::PageClass::Local) {
        return Err(denied(
            "annotations are for this session's own app preview (a page of its local server), not \
                 for web pages"
                .to_owned(),
        ));
    }
    let page = service
        .page_accessor(session_id)
        .ok_or_else(|| denied(vw_browser::NOTHING_OPEN.to_owned()))?;
    // A named target (the Debug API's way: nobody points) is checked here too; the window's pick has none.
    let named = target
        .map(|target| crate::preview_pick::annotate_target(mode, Some(target)))
        .transpose()?;
    let annotation = annotate::annotate(
        service,
        &page,
        session_id,
        mode == Some("box"),
        named.as_ref(),
    )
    .await
    .map_err(|e| CoreError::new(ErrorCode::InvalidParams, e))?;
    Ok(BrowserResult::Annotation { annotation })
}
