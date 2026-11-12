//! Preview command dispatch for the window and Debug API.
//! Depends on preview lifecycle/opening, port facts and the preview service; called by Core dispatch.
//! Ownership checks precede layout and overlay changes; replies describe the service's resulting state.

use super::{blocking, core_error, hub, open_for_session};
use crate::command::CommandResult;
use crate::core::{Core, Shared};
use crate::error::{CoreError, ErrorCode};
use crate::preview_ports::{busy_ports, busy_view, session_ports};
use crate::preview_view::{BrowserOverlayAction, PreviewRequest, PreviewResult};
use std::sync::Arc;
use vw_preview::{HistoryStep, PreviewError, PreviewService};

impl Core {
    /// Executes one preview request (the same for every origin).
    pub(crate) async fn preview(
        &self,
        request: PreviewRequest,
    ) -> Result<CommandResult, CoreError> {
        let shared = &self.shared;
        if let PreviewRequest::AppDialog { name } = request {
            shared.browser_dialogs.set_ui(name);
            return Ok(CommandResult::Preview {
                result: PreviewResult::Done {},
            });
        }
        let hub = hub(shared)?;
        let service = &hub.service;
        let err = |e: PreviewError| core_error(&e);
        let result = match request {
            PreviewRequest::AppDialog { name } => {
                shared.browser_dialogs.set_ui(name);
                PreviewResult::Done {}
            }
            PreviewRequest::List { session_id } => self.preview_list(&session_id).await?,
            PreviewRequest::Open {
                session_id,
                url,
                port,
                path,
            } => PreviewResult::Preview {
                preview: Some(
                    open_for_session(shared, &session_id, (url, port, path), false).await?,
                ),
            },
            PreviewRequest::Back { session_id } => {
                history(service, &session_id, HistoryStep::Back).await?
            }
            PreviewRequest::Forward { session_id } => {
                history(service, &session_id, HistoryStep::Forward).await?
            }
            PreviewRequest::Reload { session_id } => reload(shared, service, session_id).await?,
            PreviewRequest::SetWidth {
                session_id,
                tab_id,
                width,
            } => set_width(shared, service, &session_id, tab_id.as_deref(), width).await?,
            PreviewRequest::Layout {
                session_id,
                tab_id,
                rect,
            } => layout(shared, service, session_id, tab_id, rect).await?,
            PreviewRequest::Visibility { report } => {
                service.report_view(report, |id| crate::new_chat::is_draft(shared, id));
                PreviewResult::Done {}
            }
            PreviewRequest::NativeWindowVisibility { visible } => {
                service.report_native_window_visibility(visible);
                PreviewResult::Done {}
            }
            PreviewRequest::BrowserOverlay {
                session_id,
                tab_id,
                action,
            } => overlay(shared, service, session_id, tab_id, action).await?,
            PreviewRequest::Close { session_id } => {
                service.close(&session_id).await.map_err(err)?;
                PreviewResult::Preview {
                    preview: service.view(&session_id),
                }
            }
            PreviewRequest::Screenshot { session_id } => screenshot(service, &session_id).await?,
            PreviewRequest::Console { session_id, limit } => {
                console(service, &session_id, limit).await?
            }
            PreviewRequest::Explain { session_id, port } => PreviewResult::Explanation {
                explanation: self.explain_for_preview(&session_id, port).await?,
            },
            PreviewRequest::StopOwner { session_id, port } => {
                self.stop_busy_owner(&session_id, port).await?;
                PreviewResult::Done {}
            }
            PreviewRequest::Pick {
                session_id,
                target,
                tab_id,
            } => PreviewResult::Picked {
                picked: crate::preview_pick::run(service, &session_id, target, tab_id).await?,
            },
            PreviewRequest::CancelPick { session_id } => {
                crate::preview_pick::cancel(service, &session_id).await;
                PreviewResult::Done {}
            }
            PreviewRequest::CopyText {
                session_id,
                tab_id,
                target,
            } => PreviewResult::Copied {
                text: crate::preview_pick::copy_text(service, &session_id, &tab_id, &target)
                    .await?,
            },
        };
        Ok(CommandResult::Preview { result })
    }

    async fn preview_list(&self, session_id: &str) -> Result<PreviewResult, CoreError> {
        if !crate::new_chat::known_owner(&self.shared, session_id) {
            return Err(CoreError::not_loaded(session_id));
        }
        let hub = hub(&self.shared)?;
        let (shared, id) = (self.shared.clone(), session_id.to_owned());
        let (ports, busy) = blocking(move || {
            session_ports(&shared, &id).map(|ports| (ports, busy_ports(&shared, &id)))
        })
        .await?
        .map_err(|e| CoreError::new(ErrorCode::Internal, format!("process monitor: {e}")))?;
        if let Some(port) = hub.service.local_server_port(session_id) {
            let listening = ports.targets.iter().any(|t| t.port == port && t.listening);
            let _ = hub
                .service
                .set_local_server_listening(session_id, port, listening);
        }
        // A list from the window keeps what the user may navigate to; it never widens an agent-opened preview's
        // set beyond proven ports, because the set is replaced only by an open.
        Ok(PreviewResult::Previews {
            preview: hub.service.view(session_id),
            targets: ports.targets,
            leased_ports: ports.leased,
            busy,
        })
    }

    async fn explain_for_preview(
        &self,
        session_id: &str,
        port: u16,
    ) -> Result<crate::preview_view::BusyPortView, CoreError> {
        let explanation = self.explain_port(Some(session_id.to_owned()), port).await?;
        Ok(busy_view(&self.shared, session_id, &explanation, None))
    }
}

/// Converts the service screenshot into the command wire result.
async fn screenshot(
    service: &PreviewService,
    session_id: &str,
) -> Result<PreviewResult, CoreError> {
    let shot = service
        .screenshot(session_id)
        .await
        .map_err(|error| core_error(&error))?;
    Ok(PreviewResult::Screenshot {
        png_base64: shot.base64(),
        width: shot.width,
        height: shot.height,
    })
}

/// Bounds the requested console count before reading the service's retained entries.
async fn console(
    service: &PreviewService,
    session_id: &str,
    limit: Option<u32>,
) -> Result<PreviewResult, CoreError> {
    let limit = limit.and_then(|value| usize::try_from(value).ok());
    let (entries, dropped) = service
        .console(session_id, limit)
        .await
        .map_err(|error| core_error(&error))?;
    Ok(PreviewResult::Console { entries, dropped })
}

/// Moves history and returns the same page's resulting view.
async fn history(
    service: &PreviewService,
    session_id: &str,
    step: HistoryStep,
) -> Result<PreviewResult, CoreError> {
    service
        .history(session_id, step)
        .await
        .map_err(|error| core_error(&error))?;
    Ok(PreviewResult::Preview {
        preview: service.view(session_id),
    })
}

/// Changes width only after validating the chat and its selected page.
async fn set_width(
    shared: &Arc<Shared>,
    service: &PreviewService,
    session_id: &str,
    tab_id: Option<&str>,
    width: vw_preview::DeviceWidth,
) -> Result<PreviewResult, CoreError> {
    if !crate::new_chat::known_owner(shared, session_id) {
        return Err(CoreError::not_loaded(session_id));
    }
    let page =
        crate::browser_tabs::user_page(service, session_id, Some(tab_id.unwrap_or(session_id)))?;
    service
        .set_width(&page, width)
        .await
        .map_err(|error| core_error(&error))?;
    Ok(PreviewResult::Preview {
        preview: service.view(&page),
    })
}

/// Reports placement after checking that an explicit page belongs to the selected chat.
async fn layout(
    shared: &Arc<Shared>,
    service: &PreviewService,
    session_id: Option<String>,
    tab_id: Option<String>,
    rect: Option<vw_preview::Rect>,
) -> Result<PreviewResult, CoreError> {
    if let Some(owner) = session_id.as_deref()
        && !crate::new_chat::known_owner(shared, owner)
    {
        return Err(CoreError::not_loaded(owner));
    }
    let page = match (session_id.as_deref(), tab_id) {
        (None, None) => None,
        (Some(owner), None) => Some(owner.to_owned()),
        (Some(owner), Some(key)) => {
            if service.owner_session_id(&key).as_deref() != Some(owner) {
                return Err(CoreError::new(
                    ErrorCode::Denied,
                    "tab does not belong to this session",
                ));
            }
            Some(key)
        }
        (None, Some(_)) => {
            return Err(CoreError::new(
                ErrorCode::InvalidParams,
                "tabId needs sessionId",
            ));
        }
    };
    service
        .report_tab_layout(page, rect)
        .await
        .map_err(|error| core_error(&error))?;
    Ok(PreviewResult::Done {})
}

/// Refreshes a stopped local server from fresh process facts, or reloads an ordinary page.
async fn reload(
    shared: &Arc<Shared>,
    service: &PreviewService,
    session_id: String,
) -> Result<PreviewResult, CoreError> {
    let err = |e: PreviewError| core_error(&e);
    if let Some(port) = service.local_server_port(&session_id) {
        let (shared_for_scan, owner) = (shared.clone(), session_id.clone());
        let ports = blocking(move || session_ports(&shared_for_scan, &owner))
            .await?
            .map_err(|e| CoreError::new(ErrorCode::Internal, format!("process monitor: {e}")))?;
        let listening = ports.targets.iter().any(|t| t.port == port && t.listening);
        let was_stopped = service
            .view(&session_id)
            .is_some_and(|v| v.state == vw_preview::PreviewState::Stopped);
        let _ = service.set_local_server_listening(&session_id, port, listening);
        if listening && was_stopped {
            let url = service.view(&session_id).and_then(|v| v.url);
            if let Some(url) = url {
                open_for_session(shared, &session_id, (Some(url), None, None), false).await?;
            }
        } else if listening {
            service
                .history(&session_id, HistoryStep::Reload)
                .await
                .map_err(err)?;
        }
    } else {
        service
            .history(&session_id, HistoryStep::Reload)
            .await
            .map_err(err)?;
    }
    Ok(PreviewResult::Preview {
        preview: service.view(&session_id),
    })
}

/// Parks, displays or restores the exact owned browser page for its window overlay.
async fn overlay(
    shared: &Arc<Shared>,
    service: &PreviewService,
    session_id: String,
    tab_id: Option<String>,
    action: BrowserOverlayAction,
) -> Result<PreviewResult, CoreError> {
    let err = |e: PreviewError| core_error(&e);
    if !crate::new_chat::known_owner(shared, &session_id) {
        return Err(CoreError::not_loaded(&session_id));
    }
    let page = match tab_id {
        Some(key) => {
            if service.owner_session_id(&key).as_deref() != Some(session_id.as_str()) {
                return Err(CoreError::new(
                    ErrorCode::Denied,
                    "tab does not belong to this session",
                ));
            }
            key
        }
        None => session_id.clone(),
    };
    tracing::debug!(session = %session_id, page = %page, ?action, "browser overlay request");
    Ok(match action {
        BrowserOverlayAction::Prepare => {
            match service.prepare_browser_overlay(&session_id, &page).await {
                Ok(prepared) => PreviewResult::Overlay {
                    snapshot: prepared.snapshot,
                    bounds: Some(prepared.bounds),
                },
                // A tab switch overtook the menu: nothing to park, the window closes its menus.
                Err(PreviewError::NotInView) => PreviewResult::Overlay {
                    snapshot: None,
                    bounds: None,
                },
                Err(error) => return Err(err(error)),
            }
        }
        BrowserOverlayAction::Show => {
            service
                .show_browser_overlay(&session_id, &page)
                .await
                .map_err(err)?;
            PreviewResult::Overlay {
                snapshot: None,
                bounds: None,
            }
        }
        BrowserOverlayAction::Close => {
            service
                .close_browser_overlay(&session_id, &page)
                .await
                .map_err(err)?;
            PreviewResult::Overlay {
                snapshot: None,
                bounds: None,
            }
        }
    })
}
